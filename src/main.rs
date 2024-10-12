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
use soundswitch::SoundSwitchConnector;

use serde_json;

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
    fn new(h: HANDLE, base: usize, offsets: Pointer) -> Value<T> {
        let mut address = base;

        for offset in offsets.offsets {
            address = read::<usize>(h, address + offset)
                .expect("Memory read failed, check your Rekordbox version!");
        }
        address += offsets.final_offset;

        Value::<T> {
            address,
            handle: h,
            _marker: PhantomData::<T>,
        }
    }

    fn read(&self) -> T {
        read::<T>(self.handle, self.address).unwrap()
    }

    fn read_bytes(&self, times: usize) -> Vec<u8> {
        let mut byte_vec = Vec::new();
        for _t in 0..times {
            let read_mem_bytes = read::<u8>(self.handle, self.address + (_t)).unwrap();
            byte_vec.push(read_mem_bytes);
        }
        return byte_vec;
    }
}

pub struct Rekordbox {
    master_bpm_val: Value<f32>,
    bar1_val: Value<i32>,
    beat1_val: Value<i32>,
    bar2_val: Value<i32>,
    beat2_val: Value<i32>,
    masterdeck_index_val: Value<u8>,

    deck1_time_val: Value<i32>,
    deck2_time_val: Value<i32>,
    deck1_track_id_val: Value<i32>,
    deck2_track_id_val: Value<i32>,
    api_bearer_val: Value<Vec<u8>>,

    pub beats1: i32,
    pub beats2: i32,
    pub master_beats: i32,
    pub master_bpm: f32,
    pub masterdeck_index: u8,
    pub deck1_time: i32,
    pub deck2_time: i32,
    pub deck1_track_id: i32,
    pub deck2_track_id: i32,
    pub master_time: i32,
    pub api_bearer: String,
}

impl Rekordbox {
    fn new(offsets: RekordboxOffsets) -> Self {
        let rb = Process::from_process_name("rekordbox.exe")
            .expect("Could not find Rekordbox process! ");
        let h = rb.process_handle;

        let base = rb.get_module_base("rekordbox.exe").unwrap();

        let master_bpm_val: Value<f32> = Value::new(h, base, offsets.master_bpm);

        let api_bearer_val: Value<Vec<u8>> = Value::new(h, base, offsets.api_bearer);

        let bar1_val: Value<i32> = Value::new(h, base, offsets.deck1bar);
        let beat1_val: Value<i32> = Value::new(h, base, offsets.deck1beat);
        let bar2_val: Value<i32> = Value::new(h, base, offsets.deck2bar);
        let beat2_val: Value<i32> = Value::new(h, base, offsets.deck2beat);


        let deck1_track_id_val: Value<i32> = Value::new(h, base, offsets.deck1_track_id);
        let deck1_time_val: Value<i32> = Value::new(h, base, offsets.deck1_time);
        
        let deck2_track_id_val: Value<i32> = Value::new(h, base, offsets.deck2_track_id);
        let deck2_time_val: Value<i32> = Value::new(h, base, offsets.deck2_time);

        let masterdeck_index_val: Value<u8> = Value::new(h, base, offsets.masterdeck_index);

        Self {
            master_bpm_val,
            bar1_val,
            beat1_val,
            bar2_val,
            beat2_val,

            deck1_time_val,
            deck2_time_val,

            deck1_track_id_val,
            deck2_track_id_val,
            api_bearer_val,

            masterdeck_index_val,

            beats1: -1,
            beats2: -1,
            master_bpm: 120.0,
            masterdeck_index: 0,
            master_beats: 0,
            master_time: 0,
            deck1_track_id: 0,
            deck2_track_id: 0,
            deck1_time: 0,
            deck2_time: 0,
            api_bearer: "".to_string(),
        }
    }

    fn update(&mut self) {
        self.master_bpm = self.master_bpm_val.read();
        self.beats1 = self.bar1_val.read() * 4 + self.beat1_val.read();
        self.beats2 = self.bar2_val.read() * 4 + self.beat2_val.read();
        self.masterdeck_index = self.masterdeck_index_val.read();

        self.deck1_track_id = self.deck1_track_id_val.read();
        self.deck2_track_id = self.deck2_track_id_val.read();

        self.deck1_time = self.deck1_time_val.read();
        self.deck2_time = self.deck2_time_val.read();

        if self.masterdeck_index == 0 {
            self.master_beats = self.beats1;
            self.master_time = self.deck1_time;
        } else {
            self.master_beats = self.beats2;
            self.master_time = self.deck2_time;
        };
    }

    pub fn update_api_bearer(&mut self) {
        let api_bearer_vec = self.api_bearer_val.read_bytes(32);
        self.api_bearer = match std::str::from_utf8(&api_bearer_vec) {
            Ok(v) => v.to_string(),
            Err(e) => panic!("Invalid UTF-8 sequence: {}", e),
        };
    }
}

pub struct BeatKeeper {
    rb: Option<Rekordbox>,
    last_beat: i32,
    last_time: i32,

    pub api_bearer: String,
    
    pub last_d1track: i32,
    pub last_d2track: i32,
    pub last_master_track: i32,
    pub last_master_path: String,
    pub last_master_title: String,

    pub beat_fraction: f32,
    pub last_masterdeck_index: u8,
    pub offset_micros: f32,
    pub last_bpm: f32,
    pub new_beat: bool,
    pub new_track: bool,
    pub new_time: bool,
}

impl BeatKeeper {
    pub fn new(offsets: RekordboxOffsets) -> Self {
        BeatKeeper {
            rb: Some(Rekordbox::new(offsets)),
            last_beat: 0,
            last_time: 0,
            last_d1track: 0,
            last_d2track: 0,
            last_master_track: 0,
            last_master_path: "".to_string(),
            last_master_title: "".to_string(),
            api_bearer: "".to_string(),
            beat_fraction: 1.,
            last_masterdeck_index: 0,
            offset_micros: 0.,
            last_bpm: 0.,
            new_beat: false,
            new_track: false,
            new_time: false,
        }
    }

    pub fn dummy() -> Self {
        BeatKeeper {
            rb: None,
            last_beat: 0,
            last_time: 0,
            last_d1track: 0,
            last_d2track: 0,
            last_master_track: 0,
            last_master_path: "".to_string(),
            last_master_title: "".to_string(),
            api_bearer: "".to_string(),
            beat_fraction: 1.,
            last_masterdeck_index: 0,
            offset_micros: 0.,
            last_bpm: 0.,
            new_beat: false,
            new_track: false,
            new_time: false,
        }
    }

    pub fn update(&mut self, delta: Duration) {
        if let Some(rb) = &mut self.rb {
            let beats_per_micro = rb.master_bpm / 60. / 1000000.;
            let mut master_track_changed = false;

            rb.update(); // Fetch values from rkbx memory

            if rb.masterdeck_index != self.last_masterdeck_index {
                self.last_masterdeck_index = rb.masterdeck_index;
                self.last_beat = rb.master_beats;
                if rb.masterdeck_index == 0 {
                    self.last_master_track = rb.deck1_track_id;
                } else {
                    self.last_master_track = rb.deck2_track_id;
                }
                master_track_changed = true;
            }

            if rb.deck1_track_id != self.last_d1track  {
                println!("Deck 1 track change: {}", rb.deck1_track_id);
                self.last_d1track = rb.deck1_track_id;
                if rb.masterdeck_index == 0 {
                    self.last_master_track = rb.deck1_track_id;
                    master_track_changed = true;

                }
            }

            if rb.deck2_track_id != self.last_d2track  {
                println!("Deck 2 track change: {}", rb.deck2_track_id);
                self.last_d2track = rb.deck2_track_id;
                if rb.masterdeck_index == 1 {
                    self.last_master_track = rb.deck2_track_id;
                    master_track_changed = true;
                }
            }

            if (rb.master_beats - self.last_beat).abs() > 0 {
                self.last_beat = rb.master_beats;
                self.beat_fraction = 0.;
                self.new_beat = true;
            }

            if rb.master_time != self.last_time {
                self.last_time = rb.master_time;
                self.new_time = true;
            }
            
            if master_track_changed {
                if self.last_master_track > 0 {
                    let res = new_master_track(self.last_master_track, &self.api_bearer);
                    if res["code"] != 404 {
                        self.last_master_path = res["item"]["FolderPath"].as_str().unwrap().to_string();
                        self.last_master_title = res["item"]["FileNameL"].as_str().unwrap().to_string();
                        self.new_track = true;
                    }
                }
            }

            self.beat_fraction =
                (self.beat_fraction + delta.as_micros() as f32 * beats_per_micro) % 1.;
        } else {
            self.beat_fraction = (self.beat_fraction + delta.as_secs_f32() * 130. / 60.) % 1.;
        }
    }

    pub fn update_api_bearer(&mut self) {
        if let Some(rb) = &mut self.rb {

            rb.update_api_bearer(); // Fetch values from rkbx memory
            self.api_bearer = rb.api_bearer.clone();

        }
    }

    pub fn get_beat_faction(&mut self) -> f32 {
        (self.beat_fraction
            + if let Some(rb) = &self.rb {
                let beats_per_micro = rb.master_bpm / 60. / 1000000.;
                self.offset_micros * beats_per_micro
            } else {
                0.
            }
            + 1.)
            % 1.
    }

    pub fn get_bpm_changed(&mut self) -> Option<f32> {
        if let Some(rb) = &self.rb {
            if rb.master_bpm != self.last_bpm {
                self.last_bpm = rb.master_bpm;
                return Some(rb.master_bpm);
            }
        }
        None
    }

    pub fn get_new_beat(&mut self) -> bool {
        if self.new_beat {
            self.new_beat = false;
            return true;
        }
        false
    }

    pub fn get_new_time(&mut self) -> bool {
        if self.new_time {
            self.new_time = false;
            return true;
        }
        false
    }

    pub fn get_new_master_track(&mut self) -> bool {
        if self.new_track {
            self.new_track = false;
            return true;
        }
        false
    }

    pub fn change_beat_offset(&mut self, offset: f32) {
        self.offset_micros += offset;
    }
}

const CHARS: [&str; 4] = ["|", "/", "-", "\\"];

pub fn new_master_track(track_id: i32, api_key: &String) -> serde_json::Value {
    let client = reqwest::blocking::Client::new();

    let response = client
    .get(format!("http://127.0.0.1:30001/api/v1/data/djmdContents/{}/", track_id))
    .header("User-Agent", "rekordbox/6.8.4.0001 Windows 11(64bit)")
    .header("Accept", "*/*")
    .header("Authorization", format!("Bearer {}", api_key))
    .send().expect("failed to get response").text().expect("failed to get payload");

    let res: serde_json::Value = serde_json::from_str(&response).unwrap();

    return res;

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
    versions.sort();
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
A tool for sending Rekordbox track name, time and bpm to soundswitch (based on virtualdj communication)

Flags:

 -h  Print this help
 -u  Fetch latest offset list from GitHub and exit
 -v  Rekordbox version to target, eg. 6.7.3

 -p  Change poll value

Use r to resend master path/track to soundswitch.
Use y to reset and resend master path/track to soundswitch (useful for changing to Autoloop override during a song).

Current default version: {}
Available versions:",
                                env!("CARGO_PKG_VERSION"),
                                versions[0]
                            );
                            println!("{}", versions.join(", "));

                            /*for v in  {
                                print!("{v}, ");
                            }*/
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
        "Press r to resend master path. y to reset master path. c to quit. -h flag for help and version info."
    );
    println!();

    let mut keeper = BeatKeeper::new(offsets.clone());

    let connection = SoundSwitchConnector::discover_soundswitch();
    let mut os2l_stream = SoundSwitchConnector::initial_connect(connection);
    

    // Due to Windows timers having a default resolution 0f 15.6ms, we need to use a "too high"
    // value to acheive ~60Hz
    let period = Duration::from_micros(1000000 / poll_rate);

    let mut last_instant = Instant::now();

    let mut count = 0;
    let mut step = 0;

    let mut stdout = stdout();

    let mut first_send = false;

    // Get API bearer key
    keeper.update_api_bearer();
    println!("API key: {}",keeper.api_bearer);

    println!("Entering loop");
    loop {
        let delta = Instant::now() - last_instant; // Is this timer accurate enough?
        last_instant = Instant::now();

        keeper.update(delta); // Get values, advance time


        if keeper.get_new_beat() {
            SoundSwitchConnector::send_beatpos(&mut os2l_stream, keeper.last_beat);

            if keeper.last_beat % 4 == 1 {
                SoundSwitchConnector::send_beat(&mut os2l_stream, keeper.last_beat, keeper.last_bpm);
            }

            if keeper.get_new_master_track() {
                println!("Path: {:?}", keeper.last_master_path);
                println!("Title: {:?}", keeper.last_master_title);
                SoundSwitchConnector::send_track(&mut os2l_stream, &mut keeper.last_master_path);
            }

        }

        if keeper.get_new_time() {
            if first_send == false {
                first_send = true;
            } else {
                SoundSwitchConnector::send_time(&mut os2l_stream, keeper.last_time);
            }
        }

        while let Ok(key) = rx.try_recv() {
            match key {
                99 => {
                    //"c"
                    return;
                }
                114 => {
                    //"r"
                    println!("Path: {:?}", keeper.last_master_path);
                    println!("Title: {:?}", keeper.last_master_title);
                    SoundSwitchConnector::send_track(&mut os2l_stream, &mut keeper.last_master_path);
                }
                121 => {
                    //"y"
                    println!("Resetting playing track");
                    SoundSwitchConnector::send_track(&mut os2l_stream, &mut "".to_string());
                    sleep(Duration::from_millis(50));
                    SoundSwitchConnector::send_track(&mut os2l_stream, &mut keeper.last_master_path);
                }
                _ => (),
            }
        }

        if count % 20 == 0 {
            step = (step + 1) % 4;

            print!(
                "\rRunning {} [{:02}:{:02}]  Deck {}  Frq: {: >3}Hz  ",
                CHARS[step],
                (keeper.last_time/1000) / 60,
                (keeper.last_time/1000) % 60,
                keeper.last_masterdeck_index,
                1000000 / (delta.as_micros().max(1)),
                );
            print!(
                "Master title: {}",
                keeper.last_master_title
            );
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
