use std::process::Command;
use std::{
    env,
    io::{stdout, Write},
    marker::PhantomData,
    path::Path,
    sync::mpsc::channel,
    thread::{sleep, spawn},
    time::{Duration, Instant},
};
use toy_arms::external::{read, Process};
use winapi::um::winnt::HANDLE;

mod offsets;
use offsets::{Pointer, RekordboxOffsets};

mod soundswitch;
use soundswitch::{Os2l, DECKS};

use serde_json;

/// A deck counts as playing while its time keeps changing; this long without a change means stopped.
const PLAY_TIMEOUT: Duration = Duration::from_millis(250);

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
}

impl Rekordbox {
    fn new(o: &RekordboxOffsets) -> Self {
        let rb = Process::from_process_name("rekordbox.exe")
            .expect("Could not find Rekordbox process! ");
        let handle = rb.process_handle;
        let base = rb.get_module_base("rekordbox.exe").unwrap();

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
        };
        rb.update();
        rb
    }

    fn update(&mut self) {
        let (h, b) = (self.handle, self.base);

        ensure(&mut self.master_bpm_val, h, b, &self.bpm_ptr);
        ensure(&mut self.masterdeck_index_val, h, b, &self.masterdeck_ptr);
        self.master_bpm = read_opt(&self.master_bpm_val).unwrap_or(120.0);
        self.masterdeck_index = read_opt(&self.masterdeck_index_val).unwrap_or(255);

        for i in 0..DECKS {
            ensure(&mut self.bar_val[i], h, b, &self.bar_ptr[i]);
            ensure(&mut self.beat_val[i], h, b, &self.beat_ptr[i]);
            ensure(&mut self.time_val[i], h, b, &self.time_ptr[i]);
            ensure(&mut self.track_id_val[i], h, b, &self.track_id_ptr[i]);
            if let Some(p) = &self.fader_ptr[i] {
                // Re-resolve every time: the mixer object can be (re)allocated after startup,
                // and this is only a few extra reads.
                self.fader_val[i] = Value::new(h, b, p.clone());
            }

            if let (Some(bar), Some(beat)) = (read_opt(&self.bar_val[i]), read_opt(&self.beat_val[i])) {
                self.beats[i] = bar * 4 + beat;
            }
            self.time[i] = read_opt(&self.time_val[i]).unwrap_or(0);
            self.track_id[i] = read_opt(&self.track_id_val[i]).unwrap_or(0);
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

/// Per-deck state, ready to be sent to SoundSwitch.
#[derive(Default)]
pub struct Deck {
    pub track_id: i32,
    pub path: String,
    pub title: String,
    pub time: i32,
    pub beats: i32,
    pub playing: bool,
    pub level: f32,
    last_time_change: Option<Instant>,
    retry_track_at: Option<Instant>,
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
            if id > 0 && id != deck.track_id && deck.retry_track_at.map_or(true, |t| now >= t) {
                match fetch_track(id, &self.api_bearer) {
                    Some((path, title)) => {
                        println!("\nDeck {} track change: {} ({})", i + 1, title, id);
                        deck.track_id = id;
                        deck.path = path;
                        deck.title = title;
                        deck.retry_track_at = None;
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

    pub fn take_new_beat(&mut self) -> bool {
        std::mem::replace(&mut self.new_beat, false)
    }
}

const CHARS: [&str; 4] = ["|", "/", "-", "\\"];

/// Look up a track's (folder path, file name) through Rekordbox's local API.
fn fetch_track(track_id: i32, api_key: &str) -> Option<(String, String)> {
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
    Some((
        item["FolderPath"].as_str()?.to_string(),
        item["FileNameL"].as_str()?.to_string(),
    ))
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
                        "h" => {
                            println!(
                                " - Rekordbox OS2L v{} -
A tool for sending Rekordbox deck state (track, time, bpm, channel faders) to SoundSwitch (based on VirtualDJ communication)

Flags:

 -h  Print this help
 -u  Fetch latest offset list from GitHub and exit
 -v  Rekordbox version to target, eg. 6.7.3

 -p  Change poll value

Use r to resend deck paths/tracks to soundswitch.
Use y to reset and resend deck paths/tracks to soundswitch (useful for changing to Autoloop override during a song).

Everything SoundSwitch sends back is printed and logged to soundswitch_rx.log.

Current default version: {}
Available versions:",
                                env!("CARGO_PKG_VERSION"),
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

    println!();
    println!(
        "Press r to resend deck paths. y to reset deck paths. c to quit. -h flag for help and version info."
    );
    println!();

    let mut keeper = BeatKeeper::new(offsets);
    if keeper.has_faders() {
        println!("Channel fader offsets found: sending live deck levels");
    } else {
        println!("No channel fader offsets for this version: sending master deck as level 1, others as 0");
    }

    // Get API bearer key
    keeper.update_api_bearer();
    println!("API key: {}", keeper.api_bearer);

    let mut os2l = Os2l::connect();

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

        if os2l.is_broken() {
            println!("\nReconnecting to SoundSwitch...");
            os2l = Os2l::connect(); // fresh connection: everything below is resent
        }

        // Per-deck state. Os2l only sends values that changed since last time.
        for i in 0..DECKS {
            let deck = &keeper.decks[i];
            os2l.deck_track(i, &deck.path);
            os2l.deck_play(i, deck.playing);
            os2l.deck_level(i, deck.level);
            os2l.deck_beatpos(i, deck.beats);
            os2l.deck_time(i, deck.time);
        }
        // Rekordbox only exposes the master deck's BPM
        if let Some(m) = keeper.master {
            os2l.deck_bpm(m, keeper.bpm);
        }

        if keeper.take_new_beat() {
            // Rekordbox beats are 1..4 within a bar; OS2L wants pos % 4 == 0 on the downbeat
            os2l.beat(keeper.last_beat - 1, keeper.bpm);
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
                    for i in 0..DECKS {
                        let deck = &keeper.decks[i];
                        println!("\nDeck {} {}: {:?}", i + 1, if reset { "reset" } else { "resend" }, deck.path);
                        os2l.resend_track(i, &deck.path, reset);
                    }
                }
                _ => (),
            }
        }

        if count % 20 == 0 {
            step = (step + 1) % 4;
            print!("\rRunning {} ", CHARS[step]);
            for (i, d) in keeper.decks.iter().enumerate() {
                print!(
                    "| D{}{} {} [{:02}:{:02}] lvl {:.2} ",
                    i + 1,
                    if keeper.master == Some(i) { "*" } else { " " },
                    if d.playing { "play" } else { "stop" },
                    (d.time / 1000) / 60,
                    (d.time / 1000) % 60,
                    d.level,
                );
            }
            print!("| {: >3}Hz ", 1000000 / (delta.as_micros().max(1)));
            if let Some(m) = keeper.master {
                print!("| {}   ", keeper.decks[m].title);
            }
            stdout.flush().unwrap();
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
