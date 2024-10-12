use std::{collections::HashMap, fs::File, io::Read};

impl RekordboxOffsets {
    pub fn from_lines(lines: &[String]) -> RekordboxOffsets {
        let mut rows = lines.iter();
        RekordboxOffsets {
            rbversion: rows.next().unwrap().to_string(),
            deck1bar: Pointer::from_string(rows.next().unwrap()),
            deck1beat: Pointer::from_string(rows.next().unwrap()),
            deck2bar: Pointer::from_string(rows.next().unwrap()),
            deck2beat: Pointer::from_string(rows.next().unwrap()),
            master_bpm: Pointer::from_string(rows.next().unwrap()),
            masterdeck_index: Pointer::from_string(rows.next().unwrap()),
            // track_path_baseoffset: hexparse(rows.next().unwrap()),
            deck1_track_id: Pointer::from_string(rows.next().unwrap()),
            deck2_track_id: Pointer::from_string(rows.next().unwrap()),
            api_bearer: Pointer::from_string(rows.next().unwrap()),
            deck1_time: Pointer::from_string(rows.next().unwrap()),
            deck2_time: Pointer::from_string(rows.next().unwrap()),
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
            if line.is_empty() {
                if !lines.is_empty() {
                    let o = RekordboxOffsets::from_lines(&lines);
                    map.insert(o.rbversion.clone(), o);
                    lines.clear();
                }
            } else if !line.starts_with('#') {
                lines.push(line.to_string());
            }
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
    // pub track_path_baseoffset: Pointer,
    pub deck1_track_id: Pointer,
    pub deck2_track_id: Pointer,
    pub api_bearer: Pointer,
    pub deck1_time: Pointer,
    pub deck2_time: Pointer,  // CHANGE
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
        let split = input.split(' ').map(hexparse).collect::<Vec<usize>>();
        Self::new(split[0..split.len() - 1].to_vec(), *split.last().unwrap())
    }
}

fn hexparse(input: &str) -> usize {
    usize::from_str_radix(input, 16).unwrap()
}
