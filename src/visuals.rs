//! Output for the "DJ Visuals" server: UDP, one JSON object per datagram, fire-and-forget.
//!
//! Nothing in here may block, slow down or crash the main loop: the socket is non-blocking
//! and every send error is ignored (only remembered for the status line).

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

use serde_json::json;

pub const DEFAULT_PORT: u16 = 9099;
pub const DEFAULT_TARGET: &str = "127.0.0.1:9099";

/// Resend the current master track this often, so a restarted visuals server picks it up again
/// (the receiver dedupes on file+title, so this is harmless).
const TRACK_RESEND: Duration = Duration::from_secs(5);
/// At most ~10 BPM messages per second.
const BPM_MIN_INTERVAL: Duration = Duration::from_millis(100);
/// Smaller BPM changes than this aren't sent.
const BPM_MIN_CHANGE: f32 = 0.005;
/// What the receiver accepts.
const BPM_MIN: f32 = 40.;
const BPM_MAX: f32 = 300.;

/// Parse a `host[:port]` target (port defaults to 9099). Host names are resolved once, here.
/// IPv6 literals need brackets when a port is given, e.g. `[::1]:9099`.
pub fn parse_target(s: &str) -> Result<SocketAddr, String> {
    let s = s.trim();
    if let Ok(addr) = s.parse::<SocketAddr>() {
        return Ok(addr);
    }
    let bare = s.strip_prefix('[').and_then(|r| r.strip_suffix(']')).unwrap_or(s);
    if let Ok(ip) = bare.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT));
    }
    let (host, port) = match s.rsplit_once(':') {
        Some((h, p)) => (h, p.parse::<u16>().map_err(|_| format!("invalid port in '{s}'"))?),
        None => (s, DEFAULT_PORT),
    };
    if host.is_empty() {
        return Err(format!("missing host in '{s}'"));
    }
    let addrs: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|e| format!("could not resolve '{host}': {e}"))?
        .collect();
    // Prefer IPv4: the visuals laptop is usually reached on the LAN by its IPv4 address.
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or(addrs.first())
        .copied()
        .ok_or_else(|| format!("no address found for '{host}'"))
}

/// Rekordbox's beat counter (`bar * 4 + beat`, beat 1..4) to the position in the bar, 1..4 (1 = downbeat).
/// `rem_euclid` keeps this right for the negative bars before the first downbeat of the grid.
pub fn beat_in_bar(beats: i32) -> i32 {
    (beats - 1).rem_euclid(4) + 1
}

fn valid_bpm(bpm: f32) -> Option<f32> {
    (bpm.is_finite() && (BPM_MIN..=BPM_MAX).contains(&bpm)).then_some(bpm)
}

/// f32 -> f64 with 3 decimals, so 127.99 is sent as 127.99 and not 127.98999786376953.
fn round3(v: f32) -> f64 {
    (v as f64 * 1000.).round() / 1000.
}

pub struct Visuals {
    socket: UdpSocket,
    target: SocketAddr,

    /// (deck, track_id) of the master track we last sent, and when.
    track: Option<(usize, i32)>,
    track_sent_at: Option<Instant>,
    bpm: Option<f32>,
    bpm_sent_at: Option<Instant>,
    send_failed: bool,
}

impl Visuals {
    pub fn new(target: SocketAddr) -> std::io::Result<Self> {
        let bind = if target.is_ipv4() {
            SocketAddr::new(Ipv4Addr::UNSPECIFIED.into(), 0)
        } else {
            SocketAddr::new(Ipv6Addr::UNSPECIFIED.into(), 0)
        };
        let socket = UdpSocket::bind(bind)?;
        socket.set_nonblocking(true)?;
        // Lets the target be a LAN broadcast address too (e.g. 192.168.1.255).
        let _ = socket.set_broadcast(true);
        Ok(Self {
            socket,
            target,
            track: None,
            track_sent_at: None,
            bpm: None,
            bpm_sent_at: None,
            send_failed: false,
        })
    }

    pub fn target(&self) -> SocketAddr {
        self.target
    }

    /// For the status line, e.g. "vis 127.0.0.1:9099".
    pub fn status(&self) -> String {
        format!("vis {}{}", self.target, if self.send_failed { " (send error)" } else { "" })
    }

    fn send(&mut self, msg: serde_json::Value) {
        // Fire-and-forget: a missing receiver, unreachable host or full buffer must never matter.
        self.send_failed = self.socket.send_to(msg.to_string().as_bytes(), self.target).is_err();
    }

    /// The master deck's track. Sent when the master deck or its track changes, and again every
    /// few seconds. Only call this once the deck's path has been resolved for its current track.
    pub fn master_track(&mut self, deck: usize, track_id: i32, path: &str, title: &str, artist: &str, bpm: f32) {
        if path.is_empty() {
            return;
        }
        let now = Instant::now();
        let key = (deck, track_id);
        let due = self.track != Some(key) || self.track_sent_at.map_or(true, |t| now.duration_since(t) >= TRACK_RESEND);
        if !due {
            return;
        }
        self.track = Some(key);
        self.track_sent_at = Some(now);
        self.send(json!({
            "type": "track",
            "file": path.replace('/', "\\"),
            "title": title,
            "artist": artist,
            "bpm": valid_bpm(bpm).map(round3),
            "deck": deck + 1,
        }));
    }

    /// Resend the current master track on the next `master_track` call (the r / y keys).
    pub fn resend_track(&mut self) {
        self.track_sent_at = None;
    }

    /// Master BPM: only changes of at least 0.005, at most ~10 per second, only 40..300.
    pub fn bpm(&mut self, bpm: f32) {
        let Some(bpm) = valid_bpm(bpm) else {
            return;
        };
        if self.bpm.is_some_and(|last| (last - bpm).abs() < BPM_MIN_CHANGE) {
            return;
        }
        let now = Instant::now();
        if self.bpm_sent_at.is_some_and(|t| now.duration_since(t) < BPM_MIN_INTERVAL) {
            return; // picked up on a later loop iteration, since `self.bpm` still holds the old value
        }
        self.bpm = Some(bpm);
        self.bpm_sent_at = Some(now);
        self.send(json!({ "type": "bpm", "bpm": round3(bpm) }));
    }

    /// Position in the bar, 1..4 (1 = downbeat). See `beat_in_bar`.
    pub fn beat(&mut self, beat: i32) {
        self.send(json!({ "type": "beat", "beat": beat }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn beat_in_bar_matches_rekordbox_display() {
        // Rekordbox shows bar.beat with beat 1..4; the keeper stores bar * 4 + beat.
        for bar in -3..20 {
            for beat in 1..=4 {
                assert_eq!(beat_in_bar(bar * 4 + beat), beat, "bar {bar} beat {beat}");
            }
        }
    }

    #[test]
    fn targets() {
        assert_eq!(parse_target("127.0.0.1").unwrap(), "127.0.0.1:9099".parse().unwrap());
        assert_eq!(parse_target("192.168.1.20:9100").unwrap(), "192.168.1.20:9100".parse().unwrap());
        assert_eq!(parse_target("[::1]").unwrap(), "[::1]:9099".parse().unwrap());
        assert_eq!(parse_target("localhost").unwrap().port(), 9099);
        assert!(parse_target("localhost").unwrap().is_ipv4());
        assert!(parse_target("host:abc").is_err());
        assert!(parse_target(":9099").is_err());
    }

    #[test]
    fn bpm_range() {
        assert_eq!(valid_bpm(120.), Some(120.));
        assert_eq!(valid_bpm(40.), Some(40.));
        assert_eq!(valid_bpm(300.), Some(300.));
        assert_eq!(valid_bpm(0.), None);
        assert_eq!(valid_bpm(f32::NAN), None);
        assert_eq!(valid_bpm(f32::INFINITY), None);
        assert_eq!(valid_bpm(301.), None);
        assert_eq!(round3(127.99), 127.99);
    }

    fn recv_json(rx: &UdpSocket) -> Option<serde_json::Value> {
        let mut buf = [0u8; 2048];
        rx.recv(&mut buf).ok().map(|n| serde_json::from_slice(&buf[..n]).unwrap())
    }

    #[test]
    fn sends_only_what_changed() {
        let rx = UdpSocket::bind("127.0.0.1:0").unwrap();
        rx.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let mut vis = Visuals::new(rx.local_addr().unwrap()).unwrap();

        vis.master_track(0, 7, "C:/Music/x.mp3", "x.mp3", "", 128.);
        let m = recv_json(&rx).unwrap();
        assert_eq!(m["type"], "track");
        assert_eq!(m["file"], "C:\\Music\\x.mp3");
        assert_eq!(m["title"], "x.mp3");
        assert_eq!(m["bpm"], 128.0);
        assert_eq!(m["deck"], 1);

        // Same track again: nothing until the resend interval (or `resend_track`)
        vis.master_track(0, 7, "C:/Music/x.mp3", "x.mp3", "", 128.);
        vis.resend_track();
        vis.master_track(0, 7, "C:/Music/x.mp3", "x.mp3", "", 128.);
        assert_eq!(recv_json(&rx).unwrap()["deck"], 1);
        // Master switched to deck 2
        vis.master_track(1, 7, "C:/Music/x.mp3", "x.mp3", "", 128.);
        assert_eq!(recv_json(&rx).unwrap()["deck"], 2);

        vis.bpm(0.); // invalid: skipped
        vis.bpm(128.);
        assert_eq!(recv_json(&rx).unwrap(), json!({"type": "bpm", "bpm": 128.0}));
        vis.bpm(128.001); // below the threshold
        std::thread::sleep(BPM_MIN_INTERVAL);
        vis.bpm(128.5);
        vis.bpm(129.); // rate limited
        assert_eq!(recv_json(&rx).unwrap(), json!({"type": "bpm", "bpm": 128.5}));

        vis.beat(3);
        assert_eq!(recv_json(&rx).unwrap(), json!({"type": "beat", "beat": 3}));
        assert!(recv_json(&rx).is_none());
    }
}
