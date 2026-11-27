use std::process::Command;
use std::{
    env,
    io::{stdout, Write},
    marker::PhantomData,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::channel,
    },
    thread::{sleep, spawn},
    time::{Duration, Instant},
};
use toy_arms::external::{read, Process};
use winapi::um::winnt::HANDLE;

mod offsets;
use offsets::{Pointer, RekordboxOffsets};

mod soundswitch;
use soundswitch::{SoundSwitch, DECKS};

mod visuals;
use visuals::{beat_in_bar, parse_target, Visuals};

use serde_json;

/// A deck counts as playing while its time keeps changing; this long without a change means stopped.
const PLAY_TIMEOUT: Duration = Duration::from_millis(250);

/// The first djmdContents response is saved here, to check which metadata fields Rekordbox returns.
const TRACK_JSON_LOG: &str = "djmd_track_sample.json";

extern "C" {
    fn _getch() -> core::ffi::c_char;
}

fn getch() -> i8 {
    unsafe { _getch() }
}

struct Value<T> {
    address: usize,
    handle: HANDLE,
    _marker: PhantomData<T>,
}

impl<T> Value<T> {
    fn new(h: HANDLE, base: usize, offsets: Pointer) -> Option<Value<T>> {
        let mut address = base;

        for offset in offsets.offsets {
            address = match read::<usize>(h, address + offset) {
                Ok(addr) => addr,
                Err(_) => return None, // Return None if memory read fails
            };
        }
        address += offsets.final_offset;

        Some(Value::<T> {
            address,
            handle: h,
            _marker: PhantomData::<T>,
        })
    }

    fn read(&self) -> Option<T> {
        read::<T>(self.handle, self.address).ok()
    }

    fn read_bytes(&self, times: usize) -> Option<Vec<u8>> {
        let mut byte_vec = Vec::new();
        for t in 0..times {
            let read_mem_bytes = read::<u8>(self.handle, self.address + t).ok()?;
            byte_vec.push(read_mem_bytes);
        }
        Some(byte_vec)
    }
}

/// Resolve a pointer chain if it hasn't been resolved yet (Rekordbox may not have allocated it at startup).
fn ensure<T>(slot: &mut Option<Value<T>>, h: HANDLE, base: usize, p: &Pointer) {
    if slot.is_none() {
        *slot = Value::new(h, base, p.clone());
    }
}

fn read_opt<T>(slot: &Option<Value<T>>) -> Option<T> {
    slot.as_ref().and_then(|v| v.read())
}

/// Raw values read from Rekordbox memory.
pub struct Rekordbox {
    handle: HANDLE,
    base: usize,

    bpm_ptr: Pointer,
    masterdeck_ptr: Pointer,
    bearer_ptr: Pointer,
    bar_ptr: [Pointer; DECKS],
    beat_ptr: [Pointer; DECKS],
    time_ptr: [Pointer; DECKS],
    track_id_ptr: [Pointer; DECKS],
    fader_ptr: [Option<Pointer>; DECKS],

    master_bpm_val: Option<Value<f32>>,
    masterdeck_index_val: Option<Value<u8>>,
    api_bearer_val: Option<Value<u8>>,
    bar_val: [Option<Value<i32>>; DECKS],
    beat_val: [Option<Value<i32>>; DECKS],
    time_val: [Option<Value<i32>>; DECKS],
    track_id_val: [Option<Value<i32>>; DECKS],
    fader_val: [Option<Value<f64>>; DECKS],

    pub master_bpm: f32,
    pub masterdeck_index: u8,
    pub beats: [i32; DECKS],
    pub time: [i32; DECKS],
    pub track_id: [i32; DECKS],
    pub fader: [Option<f32>; DECKS],
    pub api_bearer: String,

    /// Raw reads for the -d debug line (None = pointer didn't resolve or read failed)
    pub raw_bpm: Option<f32>,
    pub raw_masterdeck: Option<u8>,
    pub raw_bar: [Option<i32>; DECKS],
    pub raw_beat: [Option<i32>; DECKS],
    pub raw_time: [Option<i32>; DECKS],
    pub raw_track_id: [Option<i32>; DECKS],
}

impl Rekordbox {
    fn new(o: &RekordboxOffsets) -> Self {
        // Wait for Rekordbox instead of exiting, so the tool can be started first
        let mut waiting = false;
        let (handle, base) = loop {
            let found = Process::from_process_name("rekordbox.exe")
                .ok()
                .and_then(|rb| rb.get_module_base("rekordbox.exe").ok().map(|base| (rb.process_handle, base)));
            if let Some(found) = found {
                if waiting {
                    println!("Found Rekordbox");
                }
                break found;
            }
            if !waiting {
                println!("Waiting for Rekordbox (rekordbox.exe) to start... Ctrl+C to quit.");
                waiting = true;
            }
            sleep(Duration::from_secs(1));
        };

        let mut rb = Self {
            handle,
            base,

            bpm_ptr: o.master_bpm.clone(),
            masterdeck_ptr: o.masterdeck_index.clone(),
            bearer_ptr: o.api_bearer.clone(),
            bar_ptr: [o.deck1bar.clone(), o.deck2bar.clone()],
            beat_ptr: [o.deck1beat.clone(), o.deck2beat.clone()],
            time_ptr: [o.deck1_time.clone(), o.deck2_time.clone()],
            track_id_ptr: [o.deck1_track_id.clone(), o.deck2_track_id.clone()],
            fader_ptr: [o.deck1_fader.clone(), o.deck2_fader.clone()],

            master_bpm_val: None,
            masterdeck_index_val: None,
            api_bearer_val: None,
            bar_val: std::array::from_fn(|_| None),
            beat_val: std::array::from_fn(|_| None),
            time_val: std::array::from_fn(|_| None),
            track_id_val: std::array::from_fn(|_| None),
            fader_val: std::array::from_fn(|_| None),

            master_bpm: 120.0,
            masterdeck_index: 255,
            beats: [-1; DECKS],
            time: [0; DECKS],
            track_id: [0; DECKS],
            fader: [None; DECKS],
            api_bearer: String::new(),

            raw_bpm: None,
            raw_masterdeck: None,
            raw_bar: [None; DECKS],
            raw_beat: [None; DECKS],
            raw_time: [None; DECKS],
            raw_track_id: [None; DECKS],
        };
        rb.update();
        rb
    }

    fn update(&mut self) {
        let (h, b) = (self.handle, self.base);

        // Re-resolve every pointer chain on every update: Rekordbox reallocates objects (e.g. the
        // deck objects when a track is loaded), and a cached address then goes stale for good.
        // It's only a few dozen small reads per update.
        self.master_bpm_val = Value::new(h, b, self.bpm_ptr.clone());
        self.masterdeck_index_val = Value::new(h, b, self.masterdeck_ptr.clone());
        self.raw_bpm = read_opt(&self.master_bpm_val);
        self.raw_masterdeck = read_opt(&self.masterdeck_index_val);
        self.master_bpm = self.raw_bpm.unwrap_or(120.0);
        self.masterdeck_index = self.raw_masterdeck.unwrap_or(255);

        for i in 0..DECKS {
            self.bar_val[i] = Value::new(h, b, self.bar_ptr[i].clone());
            self.beat_val[i] = Value::new(h, b, self.beat_ptr[i].clone());
            self.time_val[i] = Value::new(h, b, self.time_ptr[i].clone());
            self.track_id_val[i] = Value::new(h, b, self.track_id_ptr[i].clone());
            if let Some(p) = &self.fader_ptr[i] {
                self.fader_val[i] = Value::new(h, b, p.clone());
            }

            self.raw_bar[i] = read_opt(&self.bar_val[i]);
            self.raw_beat[i] = read_opt(&self.beat_val[i]);
            if let (Some(bar), Some(beat)) = (self.raw_bar[i], self.raw_beat[i]) {
                self.beats[i] = bar * 4 + beat;
            }
            self.raw_time[i] = read_opt(&self.time_val[i]);
            self.raw_track_id[i] = read_opt(&self.track_id_val[i]);
            self.time[i] = self.raw_time[i].unwrap_or(0);
            self.track_id[i] = self.raw_track_id[i].unwrap_or(0);
            // Channel fader position is stored as an f64, linear 0..1
            self.fader[i] = read_opt(&self.fader_val[i]).map(|f| f as f32).filter(|f| f.is_finite());
        }
    }

    pub fn update_api_bearer(&mut self) {
        ensure(&mut self.api_bearer_val, self.handle, self.base, &self.bearer_ptr);
        if let Some(bytes) = self.api_bearer_val.as_ref().and_then(|v| v.read_bytes(32)) {
            self.api_bearer = std::str::from_utf8(&bytes).map(|s| s.to_string()).unwrap_or_default();
        }
    }
}

/// Per-deck state, ready to be sent to SoundSwitch and the visuals.
#[derive(Default)]
pub struct Deck {
    /// Track whose path/title/artist below have been resolved through the API
    pub track_id: i32,
    /// Track currently loaded according to Rekordbox memory (may still be waiting for the API)
    pub loaded_id: i32,
    pub path: String,
    /// Track title from Rekordbox, or the file name if it has none
    pub title: String,
    pub artist: String,
    pub time: i32,
    pub beats: i32,
    pub playing: bool,
    pub level: f32,
    last_time_change: Option<Instant>,
    retry_track_at: Option<Instant>,
}

impl Deck {
    /// The path/title/artist belong to the track that is loaded right now.
    pub fn track_resolved(&self) -> bool {
        self.track_id > 0 && self.track_id == self.loaded_id && !self.path.is_empty()
    }
}

pub struct BeatKeeper {
    rb: Rekordbox,
    pub api_bearer: String,
    pub decks: [Deck; DECKS],
    pub master: Option<usize>,
    pub bpm: f32,
    pub last_beat: i32,
    new_beat: bool,
}

impl BeatKeeper {
    pub fn new(offsets: &RekordboxOffsets) -> Self {
        BeatKeeper {
            rb: Rekordbox::new(offsets),
            api_bearer: String::new(),
            decks: Default::default(),
            master: None,
            bpm: 120.,
            last_beat: 0,
            new_beat: false,
        }
    }

    pub fn has_faders(&self) -> bool {
        self.rb.fader_ptr.iter().all(|p| p.is_some())
    }

    pub fn update(&mut self) {
        self.rb.update();
        let now = Instant::now();
        self.bpm = self.rb.master_bpm;

        // Master deck and master beat counting (for the standard OS2L "beat" message)
        let idx = self.rb.masterdeck_index as usize;
        let new_master = if idx < DECKS { Some(idx) } else { self.master };
        if new_master != self.master {
            // Re-anchor on the new master so the switch itself doesn't count as a beat
            self.master = new_master;
            if let Some(m) = new_master {
                self.last_beat = self.rb.beats[m];
            }
        } else if let Some(m) = self.master {
            if self.rb.beats[m] != self.last_beat {
                self.last_beat = self.rb.beats[m];
                self.new_beat = true;
            }
        }

        let mut refresh_bearer = false;
        for i in 0..DECKS {
            let deck = &mut self.decks[i];

            let id = self.rb.track_id[i];
            deck.loaded_id = id;
            if id > 0 && id != deck.track_id && deck.retry_track_at.map_or(true, |t| now >= t) {
                match fetch_track(id, &self.api_bearer) {
                    Some(track) => {
                        deck.track_id = id;
                        deck.path = track.path;
                        deck.title = track.title.unwrap_or(track.file_name);
                        deck.artist = track.artist.unwrap_or_default();
                        deck.retry_track_at = None;
                        if deck.artist.is_empty() {
                            println!("\nDeck {} track change: {} ({})", i + 1, deck.title, id);
                        } else {
                            println!("\nDeck {} track change: {} - {} ({})", i + 1, deck.artist, deck.title, id);
                        }
                    }
                    None => {
                        // Bearer may have changed, or the API isn't up yet; try again in a second
                        deck.retry_track_at = Some(now + Duration::from_secs(1));
                        refresh_bearer = true;
                    }
                }
            }

            let t = self.rb.time[i];
            if t != deck.time {
                deck.time = t;
                deck.last_time_change = Some(now);
            }
            deck.playing = deck.last_time_change.map_or(false, |c| now.duration_since(c) < PLAY_TIMEOUT);
            deck.beats = self.rb.beats[i];

            deck.level = match self.rb.fader[i] {
                Some(f) => f.clamp(0., 1.),
                // No fader offsets for this version: fall back to "master deck is the one that's audible"
                None => {
                    if self.master.map_or(true, |m| m == i) {
                        1.
                    } else {
                        0.
                    }
                }
            };
        }
        if refresh_bearer {
            self.update_api_bearer();
        }
    }

    pub fn update_api_bearer(&mut self) {
        self.rb.update_api_bearer();
        self.api_bearer = self.rb.api_bearer.clone();
    }

    /// One line of raw Rekordbox values, for checking offsets (-d).
    pub fn debug_line(&self) -> String {
        fn o<T: std::fmt::Display>(v: Option<T>) -> String {
            v.map_or("?".to_string(), |v| v.to_string())
        }
        let rb = &self.rb;
        let mut line = format!(
            "[debug] master idx {} bpm {}",
            o(rb.raw_masterdeck),
            rb.raw_bpm.map_or("?".to_string(), |b| format!("{b:.2}"))
        );
        for i in 0..DECKS {
            line += &format!(
                " | D{} bar {} beat {} time {}ms id {} fader {}",
                i + 1,
                o(rb.raw_bar[i]),
                o(rb.raw_beat[i]),
                o(rb.raw_time[i]),
                o(rb.raw_track_id[i]),
                rb.fader[i].map_or("?".to_string(), |f| format!("{f:.3}")),
            );
        }
        line
    }

    pub fn take_new_beat(&mut self) -> bool {
        std::mem::replace(&mut self.new_beat, false)
    }
}

const CHARS: [&str; 4] = ["|", "/", "-", "\\"];

/// Visible width of the console window in characters, or None if stdout isn't a console.
fn console_width() -> Option<usize> {
    use winapi::um::processenv::GetStdHandle;
    use winapi::um::winbase::STD_OUTPUT_HANDLE;
    use winapi::um::wincon::{GetConsoleScreenBufferInfo, CONSOLE_SCREEN_BUFFER_INFO};
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(GetStdHandle(STD_OUTPUT_HANDLE), &mut info) == 0 {
            return None;
        }
        // Lines wrap at the buffer width; the window can be narrower than that (old conhost)
        let window = info.srWindow.Right as i32 - info.srWindow.Left as i32 + 1;
        let width = window.min(info.dwSize.X as i32);
        (width > 0).then_some(width as usize)
    }
}

/// Cut `line` to fit the console and pad it with spaces, so it overwrites the previous status
/// line in place (with \r) instead of wrapping onto a new line.
fn fit_to_console(line: &str) -> String {
    let Some(width) = console_width() else {
        return line.to_string();
    };
    // Leave the last column empty: writing into it makes some consoles wrap anyway
    let max = width.saturating_sub(1);
    let mut out: String = line.chars().take(max).collect();
    let len = out.chars().count();
    out.extend(std::iter::repeat(' ').take(max - len));
    out
}

/// SoundSwitch messages that are not sent. Set by the mode (legacy by default, -f for fader
/// mode) and by -x for troubleshooting. All false sends everything.
#[derive(Default, Clone, Copy)]
struct SsSkip {
    beat: bool,
    play: bool,
    level: bool,
    bpm: bool,
    time: bool,
    beatpos: bool,
    /// Send only the master deck, as deck 1 (like v0.0.3)
    deck1: bool,
}

impl SsSkip {
    /// What v0.0.3 sent, the default mode: only the master deck's track, time and beatpos, as deck 1.
    /// Sending both decks and the channel faders (-f) made SoundSwitch stop following some users' sets.
    fn legacy() -> Self {
        Self {
            deck1: true,
            play: true,
            level: true,
            bpm: true,
            beat: true,
            ..Self::default()
        }
    }

    /// Skip everything that either one skips.
    fn or(self, o: Self) -> Self {
        Self {
            beat: self.beat || o.beat,
            play: self.play || o.play,
            level: self.level || o.level,
            bpm: self.bpm || o.bpm,
            time: self.time || o.time,
            beatpos: self.beatpos || o.beatpos,
            deck1: self.deck1 || o.deck1,
        }
    }

    fn parse(list: &str) -> Self {
        let mut s = Self::default();
        for item in list.split(',').map(|i| i.trim().to_lowercase()).filter(|i| !i.is_empty()) {
            match item.as_str() {
                "beat" => s.beat = true,
                "play" => s.play = true,
                "level" => s.level = true,
                "bpm" => s.bpm = true,
                "time" => s.time = true,
                "beatpos" => s.beatpos = true,
                "deck1" => s.deck1 = true,
                // Only what v0.0.3 sent: track, time and beatpos of the master deck as deck 1
                "v3" => s = s.or(Self::legacy()),
                other => println!("-x: unknown item '{other}' (use beat, play, level, bpm, time, beatpos, deck1, v3)"),
            }
        }
        s
    }

    fn describe(&self) -> String {
        let names = [
            (self.beat, "beat"),
            (self.play, "play"),
            (self.level, "level"),
            (self.bpm, "bpm"),
            (self.time, "time"),
            (self.beatpos, "beatpos"),
            (self.deck1, "deck1"),
        ];
        names.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect::<Vec<_>>().join(",")
    }
}

/// A track as returned by Rekordbox's local API.
struct TrackMeta {
    /// Full path to the file (FolderPath)
    path: String,
    /// FileNameL
    file_name: String,
    title: Option<String>,
    artist: Option<String>,
}

/// A non-empty string field, if there is one.
fn text_field(v: &serde_json::Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// Look up a track's path and metadata through Rekordbox's local API.
fn fetch_track(track_id: i32, api_key: &str) -> Option<TrackMeta> {
    let client = reqwest::blocking::Client::new();
    let response = client
        .get(format!("http://127.0.0.1:30001/api/v1/data/djmdContents/{}/", track_id))
        .header("User-Agent", "rekordbox/6.8.4.0001 Windows 11(64bit)")
        .header("Accept", "*/*")
        .header("Authorization", format!("Bearer {}", api_key))
        .timeout(Duration::from_secs(2))
        .send()
        .ok()?
        .text()
        .ok()?;
    let res: serde_json::Value = serde_json::from_str(&response).ok()?;
    let item = &res["item"];
    let track = TrackMeta {
        path: item["FolderPath"].as_str()?.to_string(),
        file_name: item["FileNameL"].as_str()?.to_string(),
        // Same column name as in Rekordbox's djmdContent table
        title: text_field(&item["Title"]),
        // djmdContent itself only has an ArtistID; use a name only if the API includes one.
        // Check TRACK_JSON_LOG for the real field names.
        artist: text_field(&item["ArtistName"]).or_else(|| text_field(&item["Artist"]["Name"])),
    };
    log_track_json_once(&res);
    Some(track)
}

/// Save the first successful djmdContents response (once per run) to see the real field names.
fn log_track_json_once(res: &serde_json::Value) {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    let pretty = serde_json::to_string_pretty(res).unwrap_or_default();
    let keys: Vec<&str> = res["item"].as_object().map(|o| o.keys().map(String::as_str).collect()).unwrap_or_default();
    match std::fs::write(TRACK_JSON_LOG, pretty) {
        Ok(()) => println!("\nSaved the raw track JSON to {TRACK_JSON_LOG}. Fields: {}", keys.join(", ")),
        Err(e) => println!("\nCould not write {TRACK_JSON_LOG} ({e}). Track fields: {}", keys.join(", ")),
    }
}

fn main() {
    if !Path::new("./offsets").exists() {
        println!("Offsets not found, downloading from repo...");
        download_offsets();
    }

    let (tx, rx) = channel::<i8>();
    spawn(move || loop {
        tx.send(getch()).unwrap();
    });

    let args: Vec<String> = env::args().collect();

    let version_offsets = RekordboxOffsets::from_file("offsets");
    let mut versions: Vec<String> = version_offsets.keys().map(|x| x.to_string()).collect();
    // Sort numerically so e.g. 7.2.19 > 7.2.8
    versions.sort_by_key(|v| v.split('.').map(|p| p.parse::<u32>().unwrap_or(0)).collect::<Vec<u32>>());
    versions.reverse();
    let mut target_version = versions[0].clone();
    let mut poll_rate: u64 = 60;
    // DJ Visuals is off unless a target is given with -t
    let mut visuals_target = "off".to_string();
    let mut use_soundswitch = true;
    let mut debug = false;
    let mut ss_skip = SsSkip::default();
    // Both decks + channel faders to SoundSwitch (-f). Off by default: legacy master-deck-only mode.
    let mut fader_mode = false;

    let mut args_iter = args.iter();
    args_iter.next();
    while let Some(arg) = args_iter.next() {
        let mut chars = arg.chars();
        if let Some(char) = chars.next() {
            if char == '-' {
                if let Some(flag) = chars.next() {
                    match flag.to_string().as_str() {
                        "u" => {
                            println!("Updating offsets...");
                            download_offsets();
                            return;
                        }
                        "p" => {
                            if let Some(poll_arg) = args_iter.next() {
                                match poll_arg.parse::<u64>() {
                                    Ok(value) => {
                                        poll_rate = value; // Update poll_rate if parsing is successful
                                    }
                                    Err(_) => {
                                        println!("Invalid input for poll_rate. Using default value: {}", poll_rate);
                                    }
                                }
                            }
                        }
                        "v" => {
                            target_version = args_iter.next().unwrap().to_string();
                        }
                        "t" => match args_iter.next() {
                            Some(t) => visuals_target = t.to_string(),
                            None => println!("-t needs a value (host[:port] or off). Using {}", visuals_target),
                        },
                        "n" => {
                            use_soundswitch = false;
                        }
                        "d" => {
                            debug = true;
                        }
                        "f" => {
                            fader_mode = true;
                        }
                        "x" => match args_iter.next() {
                            Some(list) => ss_skip = SsSkip::parse(list),
                            None => println!("-x needs a list, eg. -x beat,play"),
                        },
                        "h" => {
                            println!(
                                " - Rekordbox OS2L v{} -
A tool for sending Rekordbox deck state (track, time, beat position) to SoundSwitch (based on VirtualDJ communication),
and the master track, BPM and beats to a DJ Visuals server over UDP.

Flags:

 -h  Print this help
 -u  Fetch latest offset list from GitHub and exit
 -v  Rekordbox version to target, eg. 6.7.3

 -p  Change poll value

 -t  Send to a DJ Visuals server at host[:port] (port defaults to {}), eg. {} for this machine,
     192.168.1.50 or visuals.local:9100. Off by default.
 -n  Run without SoundSwitch (no discovery/connect), eg. to only feed DJ Visuals
 -f  Fader mode: send both decks to SoundSwitch, with channel faders, play state and BPM.
     Default (without -f) is the classic mode: only the master deck, sent as deck 1.
     Fader mode needs \"External mixer\" turned off in SoundSwitch's settings. If you mix with
     the channel faders (not the crossfader), also turn on \"Upfader only\", or the lights only reach half brightness.
 -d  Print the raw values read from Rekordbox once a second (for checking offsets; ? = not readable)
 -x  Don't send some SoundSwitch messages (for troubleshooting, mostly with -f), comma separated:
     beat, play, level, bpm, time, beatpos, deck1 (only the master deck, as deck 1),
     or v3 (only what v0.0.3 sent: master deck track/time/beatpos as deck 1)

Use r to resend deck paths/tracks to soundswitch (and the master track to DJ Visuals).
Use y to reset and resend deck paths/tracks to soundswitch (useful for changing to Autoloop override during a song).

Everything SoundSwitch sends back is printed and logged to soundswitch_rx.log.

Current default version: {}
Available versions:",
                                env!("CARGO_PKG_VERSION"),
                                visuals::DEFAULT_PORT,
                                visuals::DEFAULT_TARGET,
                                versions[0]
                            );
                            println!("{}", versions.join(", "));
                            println!();
                            return;
                        }

                        c => {
                            println!("Unknown flag -{c}");
                        }
                    }
                }
            }
        }
    }

    let offsets = if let Some(offsets) = version_offsets.get(target_version.as_str()) {
        offsets
    } else {
        println!("Unsupported version! {target_version}");
        return;
    };
    println!("Targeting Rekordbox version {target_version}");

    // DJ Visuals output (UDP, fire-and-forget)
    let mut visuals = if visuals_target.eq_ignore_ascii_case("off") {
        println!("DJ Visuals output off (use -t host[:port] to enable)");
        None
    } else {
        match parse_target(&visuals_target).and_then(|addr| Visuals::new(addr).map_err(|e| e.to_string())) {
            Ok(v) => {
                println!("Sending to DJ Visuals at {} (UDP)", v.target());
                Some(v)
            }
            Err(e) => {
                println!("DJ Visuals output disabled: bad target '{visuals_target}': {e}");
                None
            }
        }
    };
    if !use_soundswitch && visuals.is_none() {
        println!("Nothing to send to: SoundSwitch is off (-n) and DJ Visuals has no target (use -t host[:port]).");
        return;
    }

    println!();
    println!(
        "Press r to resend deck paths. y to reset deck paths. c to quit. -h flag for help and version info."
    );
    println!();

    let mut keeper = BeatKeeper::new(offsets);
    if use_soundswitch {
        if fader_mode {
            println!("SoundSwitch: fader mode (-f), sending both decks");
            if keeper.has_faders() {
                println!("Channel fader offsets found: sending live deck levels");
            } else {
                println!("No channel fader offsets for this version: sending master deck as level 1, others as 0");
            }
        } else {
            println!("SoundSwitch: classic mode, sending the master deck as deck 1 (use -f for both decks + faders)");
        }
    }

    // Get API bearer key
    keeper.update_api_bearer();
    println!("API key: {}", keeper.api_bearer);

    // Discovery and (re)connecting run in the background, so the loop (and the visuals) keep going meanwhile.
    let mut soundswitch = if use_soundswitch {
        Some(SoundSwitch::start())
    } else {
        println!("SoundSwitch disabled (-n)");
        None
    };
    let skipped = ss_skip.describe();
    if use_soundswitch && !skipped.is_empty() {
        println!("Not sending to SoundSwitch (-x): {skipped}");
    }
    if !fader_mode {
        ss_skip = ss_skip.or(SsSkip::legacy());
    }

    // Due to Windows timers having a default resolution 0f 15.6ms, we need to use a "too high"
    // value to acheive ~60Hz
    let period = Duration::from_micros(1000000 / poll_rate);

    let mut last_instant = Instant::now();
    let mut count = 0;
    let mut step = 0;
    let mut stdout = stdout();

    println!("Entering loop");
    loop {
        let delta = Instant::now() - last_instant;
        last_instant = Instant::now();

        keeper.update(); // Read values from Rekordbox memory
        let new_beat = keeper.take_new_beat();

        if let Some(os2l) = soundswitch.as_mut().and_then(SoundSwitch::poll) {
            // (SoundSwitch deck, Rekordbox deck) pairs
            let decks: Vec<(usize, usize)> = if ss_skip.deck1 {
                keeper.master.map(|m| vec![(0, m)]).unwrap_or_default()
            } else {
                (0..DECKS).map(|i| (i, i)).collect()
            };
            // Per-deck state. Os2l only sends values that changed since last time.
            for (out, i) in decks {
                let deck = &keeper.decks[i];
                os2l.deck_track(out, &deck.path);
                if !ss_skip.play {
                    os2l.deck_play(out, deck.playing);
                }
                if !ss_skip.level {
                    os2l.deck_level(out, deck.level);
                }
                if !ss_skip.beatpos {
                    os2l.deck_beatpos(out, deck.beats);
                }
                if !ss_skip.time {
                    os2l.deck_time(out, deck.time);
                }
            }
            // Rekordbox only exposes the master deck's BPM
            if let Some(m) = keeper.master {
                if !ss_skip.bpm {
                    os2l.deck_bpm(if ss_skip.deck1 { 0 } else { m }, keeper.bpm);
                }
            }

            if new_beat && !ss_skip.beat {
                // Rekordbox beats are 1..4 within a bar; OS2L wants pos % 4 == 0 on the downbeat
                os2l.beat(keeper.last_beat - 1, keeper.bpm);
            }
        }

        if let Some(vis) = visuals.as_mut() {
            if let Some(m) = keeper.master {
                // Only the track that is playing out loud (the master deck's), not whatever gets loaded on the other deck
                let deck = &keeper.decks[m];
                if deck.track_resolved() {
                    vis.master_track(m, deck.track_id, &deck.path, &deck.title, &deck.artist, keeper.bpm);
                }
            }
            vis.bpm(keeper.bpm);
            // Only while the master deck plays, so scrubbing/cueing a stopped deck doesn't jerk the visuals
            if new_beat && keeper.master.is_some_and(|m| keeper.decks[m].playing) {
                vis.beat(beat_in_bar(keeper.last_beat));
            }
        }

        while let Ok(key) = rx.try_recv() {
            match key {
                99 => {
                    //"c"
                    return;
                }
                114 | 121 => {
                    //"r" resend, "y" reset + resend
                    let reset = key == 121;
                    match soundswitch.as_mut().and_then(SoundSwitch::connection) {
                        Some(os2l) => {
                            let decks: Vec<(usize, usize)> = if ss_skip.deck1 {
                                keeper.master.map(|m| vec![(0, m)]).unwrap_or_default()
                            } else {
                                (0..DECKS).map(|i| (i, i)).collect()
                            };
                            for (out, i) in decks {
                                let deck = &keeper.decks[i];
                                println!("\nDeck {} {}: {:?}", i + 1, if reset { "reset" } else { "resend" }, deck.path);
                                os2l.resend_track(out, &deck.path, reset);
                            }
                        }
                        None if use_soundswitch => println!("\nSoundSwitch is not connected yet"),
                        None => {}
                    }
                    if let Some(vis) = visuals.as_mut() {
                        vis.resend_track();
                    }
                }
                _ => (),
            }
        }

        if count % 20 == 0 {
            step = (step + 1) % 4;
            let mut line = format!("Running {} ", CHARS[step]);
            for (i, d) in keeper.decks.iter().enumerate() {
                line += &format!(
                    "| D{}{} {} [{:02}:{:02}] lvl {:.2} ",
                    i + 1,
                    if keeper.master == Some(i) { "*" } else { " " },
                    if d.playing { "play" } else { "stop" },
                    (d.time / 1000) / 60,
                    (d.time / 1000) % 60,
                    d.level,
                );
            }
            line += &format!("| {: >3}Hz ", 1000000 / (delta.as_micros().max(1)));
            line += &format!(
                "| ss {} ",
                match &soundswitch {
                    None => "off",
                    Some(s) if s.is_connected() => "on",
                    Some(_) => "wait",
                }
            );
            match &visuals {
                Some(v) => line += &format!("| {} ", v.status()),
                None => line += "| vis off ",
            }
            if let Some(m) = keeper.master {
                line += &format!("| {}", keeper.decks[m].title);
            }
            print!("\r{}", fit_to_console(&line));
            stdout.flush().unwrap();
        }
        if debug && count % 60 == 0 {
            println!("\n{}", keeper.debug_line());
        }
        count = (count + 1) % 120;

        sleep(period);
    }
}

fn download_offsets() {
    match Command::new("curl")
        .args([
            "-o",
            "offsets",
            "https://raw.githubusercontent.com/fjel/rkbx_os2l/master/offsets",
        ])
        .output()
    {
        Ok(output) => {
            println!("{}", String::from_utf8(output.stdout).unwrap());
            println!("{}", String::from_utf8(output.stderr).unwrap());
        }
        Err(error) => println!("{}", error),
    }
    println!("Done!");
}
