use std::{collections::HashMap, fs::File, io::Read};

impl RekordboxOffsets {
    pub fn from_lines(lines: &[String]) -> RekordboxOffsets {
        let mut rows = lines.iter();
        let rbversion = rows.next().expect("Empty offsets entry").trim().to_string();
        let mut next = || Pointer::from_string(rows.next().expect("Incomplete offsets entry"));
        let deck1bar = next();
        let deck1beat = next();
        let deck2bar = next();
        let deck2beat = next();
        let master_bpm = next();
        let masterdeck_index = next();
        let deck1_track_id = next();
        let deck2_track_id = next();
        let api_bearer = next();
        let deck1_time = next();
        let deck2_time = next();
        drop(next);
        // Optional (newer entries only): channel fader position per deck, f64 0..1
        let deck1_fader = rows.next().map(|s| Pointer::from_string(s));
        let deck2_fader = rows.next().map(|s| Pointer::from_string(s));
        RekordboxOffsets {
            rbversion,
            deck1bar,
            deck1beat,
            deck2bar,
            deck2beat,
            master_bpm,
            masterdeck_index,
            deck1_track_id,
            deck2_track_id,
            api_bearer,
            deck1_time,
            deck2_time,
            deck1_fader,
            deck2_fader,
        }
    }

    pub fn from_file(name: &str) -> HashMap<String, RekordboxOffsets> {
        let mut file = File::open(name).unwrap();
        let mut contents = String::new();
        file.read_to_string(&mut contents).unwrap();
        drop(file);

        let mut map = HashMap::new();

        let mut lines = vec![];
        for line in contents.lines() {
            if line.trim().is_empty() {
                if !lines.is_empty() {
                    let o = RekordboxOffsets::from_lines(&lines);
                    map.insert(o.rbversion.clone(), o);
                    lines.clear();
                }
            } else if !line.starts_with('#') {
                lines.push(line.to_string());
            }
        }

        // Handle the last entry if the file doesn't end with a blank line
        if !lines.is_empty() {
            let o = RekordboxOffsets::from_lines(&lines);
            map.insert(o.rbversion.clone(), o);
        }

        map
    }
}

#[derive(Clone)]
pub struct RekordboxOffsets {
    pub rbversion: String,
    pub deck1bar: Pointer,
    pub deck1beat: Pointer,
    pub deck2bar: Pointer,
    pub deck2beat: Pointer,
    pub master_bpm: Pointer,
    pub masterdeck_index: Pointer,
    pub deck1_track_id: Pointer,
    pub deck2_track_id: Pointer,
    pub api_bearer: Pointer,
    pub deck1_time: Pointer,
    pub deck2_time: Pointer,
    pub deck1_fader: Option<Pointer>,
    pub deck2_fader: Option<Pointer>,
}

#[derive(Clone)]
pub struct Pointer {
    pub offsets: Vec<usize>,
    pub final_offset: usize,
}

impl Pointer {
    pub fn new(offests: Vec<usize>, final_offset: usize) -> Pointer {
        Pointer {
            offsets: offests,
            final_offset,
        }
    }

    pub fn from_string(input: &str) -> Self {
        let split = input.split_whitespace().map(hexparse).collect::<Vec<usize>>();
        Self::new(split[0..split.len() - 1].to_vec(), *split.last().unwrap())
    }
}

fn hexparse(input: &str) -> usize {
    usize::from_str_radix(input, 16).unwrap_or_else(|_| panic!("Invalid hex value in offsets file: {input}"))
}
