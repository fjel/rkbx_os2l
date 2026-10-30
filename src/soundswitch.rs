use std::{
    fs::OpenOptions,
    io::{Read, Write},
    net::TcpStream,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{channel, Receiver, TryRecvError},
        Arc,
    },
    thread::{sleep, spawn},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mdns_sd::{ServiceDaemon, ServiceEvent};
use serde_json::json;

/// Number of decks we read from Rekordbox and report to SoundSwitch.
pub const DECKS: usize = 2;

/// Everything SoundSwitch sends back to us is appended here (and printed).
const RX_LOG: &str = "soundswitch_rx.log";

pub struct SoundSwitchConnectionAddr {
    pub soundswitch_ip: String,
    pub soundswitch_port: u16,
}

pub struct SoundSwitchConnector;

impl SoundSwitchConnector {
    /// Browse mDNS until SoundSwitch shows up. None if mDNS itself fails (the caller retries).
    pub fn discover_soundswitch() -> Option<SoundSwitchConnectionAddr> {
        let mdns = match ServiceDaemon::new() {
            Ok(mdns) => mdns,
            Err(e) => {
                eprintln!("\nFailed to create mDNS daemon: {e}");
                return None;
            }
        };
        let service_type = "_os2l._tcp.local.";
        let receiver = match mdns.browse(service_type) {
            Ok(receiver) => receiver,
            Err(e) => {
                eprintln!("\nFailed to browse mDNS: {e}");
                let _ = mdns.shutdown();
                return None;
            }
        };

        println!("\nLooking for SoundSwitch application...");
        while let Ok(event) = receiver.recv() {
            if let ServiceEvent::ServiceResolved(info) = event {
                let service_name = info.get_fullname();
                println!("\nResolved a new service: {}", service_name);
                if !service_name.starts_with("SoundSwitch") {
                    continue;
                }
                let Some(ip) = info.get_addresses_v4().iter().next().map(|ip| ip.to_string()) else {
                    continue;
                };
                let connection = SoundSwitchConnectionAddr {
                    soundswitch_ip: ip,
                    soundswitch_port: info.get_port(),
                };
                println!("SoundSwitch at {}:{}", connection.soundswitch_ip, connection.soundswitch_port);
                let _ = mdns.shutdown();
                return Some(connection);
            }
        }
        let _ = mdns.shutdown();
        None
    }
}

/// Owns the SoundSwitch connection. Discovery and (re)connecting happen on a background
/// thread, so a missing or restarting SoundSwitch never blocks the caller's loop.
pub struct SoundSwitch {
    conn: Option<Os2l>,
    pending: Option<Receiver<Os2l>>,
}

impl SoundSwitch {
    /// Start looking for SoundSwitch in the background.
    pub fn start() -> Self {
        let mut ss = Self { conn: None, pending: None };
        ss.spawn_connect();
        ss
    }

    fn spawn_connect(&mut self) {
        let (tx, rx) = channel();
        spawn(move || {
            let _ = tx.send(Os2l::connect());
        });
        self.pending = Some(rx);
    }

    /// Call once per loop iteration: picks up a finished background connect, starts a new one
    /// when the connection broke, and returns the live connection if there is one.
    pub fn poll(&mut self) -> Option<&mut Os2l> {
        if self.conn.as_ref().is_some_and(|c| c.is_broken()) {
            println!("\nReconnecting to SoundSwitch...");
            self.conn = None; // fresh connection: everything is resent once it's back
            self.spawn_connect();
        }
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(os2l) => {
                    self.conn = Some(os2l);
                    self.pending = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.spawn_connect(),
            }
        }
        self.conn.as_mut()
    }

    /// The live connection, if any (without starting a reconnect).
    pub fn connection(&mut self) -> Option<&mut Os2l> {
        self.conn.as_mut()
    }

    pub fn is_connected(&self) -> bool {
        self.conn.is_some()
    }
}

/// What we last told SoundSwitch about a deck, so we only send changes
/// (the same way VirtualDJ only sends "subscribed" events when a value changes).
#[derive(Default)]
struct SentDeck {
    path: Option<String>,
    time: Option<i32>,
    beatpos: Option<i32>,
    playing: Option<bool>,
    bpm: Option<f32>,
    level: Option<f32>,
    track_changes: u32,
}

/// An OS2L connection to SoundSwitch that speaks VirtualDJ's "subscribed" dialect.
pub struct Os2l {
    stream: TcpStream,
    broken: bool,
    closed_by_peer: Arc<AtomicBool>,
    sent: [SentDeck; DECKS],
    last_beat: Option<(i32, f32)>,
}

fn trigger(deck: usize, what: &str) -> String {
    format!("deck {} {}", deck + 1, what)
}

/// Whole numbers as integers (VirtualDJ sends `1`, not `1.0`), others rounded to 4 decimals.
fn num(v: f32) -> serde_json::Value {
    let v = (v as f64 * 10000.).round() / 10000.;
    if v.fract() == 0. && v.abs() < 1e9 {
        json!(v as i64)
    } else {
        json!(v)
    }
}

impl Os2l {
    /// Find SoundSwitch over mDNS and connect, retrying until it works.
    /// This blocks; use `SoundSwitch` to run it in the background.
    pub fn connect() -> Self {
        loop {
            let Some(addr) = SoundSwitchConnector::discover_soundswitch() else {
                sleep(Duration::from_secs(1));
                continue;
            };
            match TcpStream::connect((addr.soundswitch_ip.as_str(), addr.soundswitch_port)) {
                Ok(stream) => return Self::from_stream(stream),
                Err(e) => {
                    eprintln!("Could not connect to SoundSwitch ({e}), retrying...");
                    sleep(Duration::from_secs(1));
                }
            }
        }
    }

    fn from_stream(stream: TcpStream) -> Self {
        // Small, latency-sensitive messages: don't let Nagle batch them.
        let _ = stream.set_nodelay(true);
        let closed_by_peer = Arc::new(AtomicBool::new(false));
        spawn_reader(&stream, closed_by_peer.clone());
        let mut os2l = Self {
            stream,
            broken: false,
            closed_by_peer,
            sent: Default::default(),
            last_beat: None,
        };
        os2l.handshake();
        os2l
    }

    pub fn is_broken(&self) -> bool {
        self.broken || self.closed_by_peer.load(Ordering::Relaxed)
    }

    fn send_line(&mut self, msg: &str) {
        if self.broken {
            return;
        }
        let mut line = String::with_capacity(msg.len() + 1);
        line.push_str(msg);
        line.push('\n');
        if let Err(e) = self.stream.write_all(line.as_bytes()) {
            eprintln!("\nLost connection to SoundSwitch: {e}");
            self.broken = true;
        }
    }

    fn subscribed(&mut self, trigger: String, value: serde_json::Value) {
        // serde_json sorts keys, which gives the same evt/trigger/value order VirtualDJ uses.
        let msg = json!({ "evt": "subscribed", "trigger": trigger, "value": value });
        self.send_line(&msg.to_string());
    }

    /// Initial state, mirroring what VirtualDJ sends right after SoundSwitch connects.
    /// Live values (level, play, bpm, ...) are sent by the main loop right after this.
    fn handshake(&mut self) {
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} get_text '%SOUNDSWITCH_ID'\",\"value\":\"\"}}"));
        }
        sleep(Duration::from_millis(30));
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} level\",\"value\":1}}"));
        }
        self.send_line("{\"evt\":\"subscribed\",\"trigger\":\"crossfader\",\"value\":0.5}");
        sleep(Duration::from_millis(50));
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} get_bpm\",\"value\":120}}"));
        }
        sleep(Duration::from_millis(50));
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} play\",\"value\":\"off\"}}"));
        }
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} loop\",\"value\":\"off\"}}"));
        }
        sleep(Duration::from_millis(50));
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} get_loop\",\"value\":8}}"));
        }
        sleep(Duration::from_millis(50));
        for d in 1..=4 {
            let sizes = ["0.03125", "0.0625", "0.125", "0.25", "0.5", "0.75", "1", "2", "4"];
            let expr: Vec<String> = sizes
                .iter()
                .map(|s| format!("deck {d} loop_roll {s} ? constant {s}"))
                .collect();
            self.send_line(&format!(
                "{{\"evt\":\"subscribed\",\"trigger\":\"{} : constant 0\",\"value\":0}}",
                expr.join(" : ")
            ));
        }
        sleep(Duration::from_millis(50));
        for d in 1..=4 {
            self.send_line(&format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck {d} get_text '%SOUNDSWITCH_ID'\",\"value\":\"{{00000000-0000-0000-0000-000000000000}}\"}}"));
        }
        sleep(Duration::from_millis(50));
    }

    fn send_track(&mut self, deck: usize, path: &str) {
        // SoundSwitch matches on the Windows path VirtualDJ would report
        let win_path = path.replace('/', "\\");
        self.subscribed(trigger(deck, "get_filepath"), json!(win_path));
        self.sent[deck].path = Some(path.to_string());
    }

    /// Report the file loaded on a deck (only sent when it changes).
    pub fn deck_track(&mut self, deck: usize, path: &str) {
        if path.is_empty() || self.sent[deck].path.as_deref() == Some(path) {
            return;
        }
        // SoundSwitch sometimes stops following tracks after many changes;
        // an empty path every 10 changes resets it.
        self.sent[deck].track_changes += 1;
        if self.sent[deck].track_changes >= 10 {
            self.sent[deck].track_changes = 0;
            self.send_track(deck, "");
            sleep(Duration::from_millis(20));
        }
        self.send_track(deck, path);
    }

    /// Resend a deck's path, optionally clearing it first (the r / y keys).
    pub fn resend_track(&mut self, deck: usize, path: &str, reset: bool) {
        if path.is_empty() {
            return;
        }
        if reset {
            self.send_track(deck, "");
            sleep(Duration::from_millis(50));
        }
        self.send_track(deck, path);
    }

    pub fn deck_time(&mut self, deck: usize, ms: i32) {
        // Time is meaningless to SoundSwitch until it knows which file is on the deck.
        if self.sent[deck].path.is_none() || self.sent[deck].time == Some(ms) {
            return;
        }
        self.sent[deck].time = Some(ms);
        self.subscribed(trigger(deck, "get_time elapsed absolute"), json!(ms));
    }

    pub fn deck_beatpos(&mut self, deck: usize, beats: i32) {
        if beats < 0 || self.sent[deck].beatpos == Some(beats) {
            return;
        }
        self.sent[deck].beatpos = Some(beats);
        self.subscribed(trigger(deck, "get_beatpos"), json!(beats));
    }

    pub fn deck_play(&mut self, deck: usize, playing: bool) {
        if self.sent[deck].playing == Some(playing) {
            return;
        }
        self.sent[deck].playing = Some(playing);
        self.subscribed(trigger(deck, "play"), json!(if playing { "on" } else { "off" }));
    }

    pub fn deck_bpm(&mut self, deck: usize, bpm: f32) {
        if !bpm.is_finite() || bpm <= 0. {
            return;
        }
        if let Some(last) = self.sent[deck].bpm {
            if (last - bpm).abs() < 0.005 {
                return;
            }
        }
        self.sent[deck].bpm = Some(bpm);
        self.subscribed(trigger(deck, "get_bpm"), num(bpm));
    }

    /// Channel fader position, 0..1. This is what SoundSwitch uses to mix lighting between decks.
    pub fn deck_level(&mut self, deck: usize, level: f32) {
        let level = level.clamp(0., 1.);
        if let Some(last) = self.sent[deck].level {
            if (last - level).abs() < 0.002 {
                return;
            }
        }
        self.sent[deck].level = Some(level);
        self.subscribed(trigger(deck, "level"), num(level));
    }

    /// Standard OS2L beat message. `pos` should be divisible by 4 on bar boundaries.
    pub fn beat(&mut self, pos: i32, bpm: f32) {
        let change = match self.last_beat {
            Some((last_pos, last_bpm)) => pos != last_pos + 1 || (bpm - last_bpm).abs() > 0.01,
            None => true,
        };
        self.last_beat = Some((pos, bpm));
        self.send_line(&format!(
            "{{\"evt\":\"beat\",\"change\":{},\"pos\":{},\"bpm\":{},\"strength\":0}}",
            change,
            pos,
            num(bpm)
        ));
    }
}

/// Dump everything SoundSwitch sends (its "subscribe" list, feedback, ...) to the console and RX_LOG.
fn spawn_reader(stream: &TcpStream, closed: Arc<AtomicBool>) {
    let Ok(mut rd) = stream.try_clone() else {
        return;
    };
    spawn(move || {
        let mut log = OpenOptions::new().create(true).append(true).open(RX_LOG).ok();
        let mut buf = [0u8; 16384];
        loop {
            match rd.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]);
                    println!("\n[SoundSwitch -> rkbx_os2l] {}", text.trim_end());
                    if let Some(f) = log.as_mut() {
                        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
                        let _ = writeln!(f, "[{ts}] {}", text.trim_end());
                    }
                }
            }
        }
        closed.store(true, Ordering::Relaxed);
        println!("\nSoundSwitch closed the connection");
    });
}
