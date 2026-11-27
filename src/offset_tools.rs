/// Validates that offset values are within reasonable ranges
pub struct OffsetValidator {
    handle: HANDLE,
    base: usize,
}

#[derive(Debug)]
pub struct ValidationResult {
    pub name: String,
    pub value: String,
    pub valid: bool,
    pub reason: String,
}

impl OffsetValidator {
    pub fn new() -> Result<Self, String> {
        let rb = Process::from_process_name("rekordbox.exe")
            .map_err(|_| "Could not find Rekordbox process!".to_string())?;
        
        let base = rb.get_module_base("rekordbox.exe")
            .map_err(|_| "Could not get module base".to_string())?;

        Ok(OffsetValidator {
            handle: rb.process_handle,
            base,
        })
    }

    /// Follow a pointer chain and return the final address
    fn resolve_pointer(&self, pointer: &Pointer) -> Result<usize, String> {
        let mut address = self.base;

        for offset in &pointer.offsets {
            address = read::<usize>(self.handle, address + offset)
                .map_err(|_| format!("Failed to read at offset {:X}", offset))?;
        }
        address += pointer.final_offset;

        Ok(address)
    }

    /// Validate all offsets for a specific version
    pub fn validate_offsets(&self, offsets: &RekordboxOffsets) -> Vec<ValidationResult> {
        let mut results = Vec::new();

        // Validate master BPM (should be 60-220 typically)
        match self.resolve_pointer(&offsets.master_bpm) {
            Ok(addr) => {
                match read::<f32>(self.handle, addr) {
                    Ok(bpm) => {
                        results.push(ValidationResult {
                            name: "master_bpm".to_string(),
                            value: format!("{:.2}", bpm),
                            valid: bpm >= 50.0 && bpm <= 250.0 && bpm.is_finite(),
                            reason: if bpm >= 50.0 && bpm <= 250.0 && bpm.is_finite() {
                                "OK".to_string()
                            } else {
                                format!("BPM out of range or invalid (50-250)")
                            },
                        });
                    }
                    Err(_) => results.push(ValidationResult {
                        name: "master_bpm".to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    }),
                }
            }
            Err(e) => results.push(ValidationResult {
                name: "master_bpm".to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }

        // Validate masterdeck index (should be 0 or 1)
        match self.resolve_pointer(&offsets.masterdeck_index) {
            Ok(addr) => {
                match read::<u8>(self.handle, addr) {
                    Ok(idx) => {
                        results.push(ValidationResult {
                            name: "masterdeck_index".to_string(),
                            value: format!("{}", idx),
                            valid: idx <= 1,
                            reason: if idx <= 1 {
                                "OK".to_string()
                            } else {
                                "Index should be 0 or 1".to_string()
                            },
                        });
                    }
                    Err(_) => results.push(ValidationResult {
                        name: "masterdeck_index".to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    }),
                }
            }
            Err(e) => results.push(ValidationResult {
                name: "masterdeck_index".to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }

        // Validate deck bars (should be reasonable, typically < 1000)
        self.validate_bar_beat(&mut results, "deck1_bar", &offsets.deck1bar);
        self.validate_bar_beat(&mut results, "deck1_beat", &offsets.deck1beat);
        self.validate_bar_beat(&mut results, "deck2_bar", &offsets.deck2bar);
        self.validate_bar_beat(&mut results, "deck2_beat", &offsets.deck2beat);

        // Validate track IDs (should be positive or 0)
        self.validate_track_id(&mut results, "deck1_track_id", &offsets.deck1_track_id);
        self.validate_track_id(&mut results, "deck2_track_id", &offsets.deck2_track_id);

        // Validate times (should be positive)
        self.validate_time(&mut results, "deck1_time", &offsets.deck1_time);
        self.validate_time(&mut results, "deck2_time", &offsets.deck2_time);

        // Validate API bearer (should be readable string)
        match self.resolve_pointer(&offsets.api_bearer) {
            Ok(addr) => {
                let mut bytes = Vec::new();
                let mut readable = true;
                for i in 0..32 {
                    match read::<u8>(self.handle, addr + i) {
                        Ok(b) => bytes.push(b),
                        Err(_) => {
                            readable = false;
                            break;
                        }
                    }
                }
                
                if readable {
                    let preview = String::from_utf8_lossy(&bytes[..bytes.len().min(16)]);
                    let is_valid = bytes.iter().take(16).all(|&b| b >= 32 && b <= 126 || b == 0);
                    results.push(ValidationResult {
                        name: "api_bearer".to_string(),
                        value: format!("{}...", preview),
                        valid: is_valid,
                        reason: if is_valid {
                            "OK".to_string()
                        } else {
                            "Contains non-printable characters".to_string()
                        },
                    });
                } else {
                    results.push(ValidationResult {
                        name: "api_bearer".to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    });
                }
            }
            Err(e) => results.push(ValidationResult {
                name: "api_bearer".to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }

        results
    }

    fn validate_bar_beat(&self, results: &mut Vec<ValidationResult>, name: &str, pointer: &Pointer) {
        match self.resolve_pointer(pointer) {
            Ok(addr) => {
                match read::<i32>(self.handle, addr) {
                    Ok(val) => {
                        results.push(ValidationResult {
                            name: name.to_string(),
                            value: format!("{}", val),
                            valid: val >= 0 && val < 10000,
                            reason: if val >= 0 && val < 10000 {
                                "OK".to_string()
                            } else {
                                "Value out of reasonable range".to_string()
                            },
                        });
                    }
                    Err(_) => results.push(ValidationResult {
                        name: name.to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    }),
                }
            }
            Err(e) => results.push(ValidationResult {
                name: name.to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }
    }

    fn validate_track_id(&self, results: &mut Vec<ValidationResult>, name: &str, pointer: &Pointer) {
        match self.resolve_pointer(pointer) {
            Ok(addr) => {
                match read::<i32>(self.handle, addr) {
                    Ok(val) => {
                        results.push(ValidationResult {
                            name: name.to_string(),
                            value: format!("{}", val),
                            valid: val >= 0,
                            reason: if val >= 0 {
                                "OK".to_string()
                            } else {
                                "Track ID should be non-negative".to_string()
                            },
                        });
                    }
                    Err(_) => results.push(ValidationResult {
                        name: name.to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    }),
                }
            }
            Err(e) => results.push(ValidationResult {
                name: name.to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }
    }

    fn validate_time(&self, results: &mut Vec<ValidationResult>, name: &str, pointer: &Pointer) {
        match self.resolve_pointer(pointer) {
            Ok(addr) => {
                match read::<i32>(self.handle, addr) {
                    Ok(val) => {
                        // Times are in milliseconds (thousands). e.g. 30428 ~ 30.428s
                        let valid = val >= 0 && val < 3_600_000; // less than 1 hour in ms
                        results.push(ValidationResult {
                            name: name.to_string(),
                            value: format!("{} (~{:.3}s)", val, val as f32 / 1000.0),
                            valid,
                            reason: if valid {
                                "OK (milliseconds)".to_string()
                            } else {
                                "Time should be non-negative and less than 3,600,000 (ms)".to_string()
                            },
                        });
                    }
                    Err(_) => results.push(ValidationResult {
                        name: name.to_string(),
                        value: "ERROR".to_string(),
                        valid: false,
                        reason: "Failed to read value".to_string(),
                    }),
                }
            }
            Err(e) => results.push(ValidationResult {
                name: name.to_string(),
                value: "ERROR".to_string(),
                valid: false,
                reason: e,
            }),
        }
    }
}

/// Memory scanner for finding values in Rekordbox memory
pub struct MemoryScanner {
    handle: HANDLE,
    base: usize,
}

impl MemoryScanner {
    pub fn new() -> Result<Self, String> {
        let rb = Process::from_process_name("rekordbox.exe")
            .map_err(|_| "Could not find Rekordbox process!".to_string())?;
        
        let base = rb.get_module_base("rekordbox.exe")
            .map_err(|_| "Could not get module base".to_string())?;

        Ok(MemoryScanner {
            handle: rb.process_handle,
            base,
        })
    }

    /// Scan for a float value in a memory range
    pub fn scan_for_float(&self, target: f32, start_offset: usize, size: usize) -> Vec<usize> {
        let mut matches = Vec::new();
        let epsilon = 0.01; // Allow small float differences

        for offset in (start_offset..start_offset + size).step_by(4) {
            if let Ok(value) = read::<f32>(self.handle, self.base + offset) {
                if (value - target).abs() < epsilon {
                    matches.push(offset);
                }
            }
        }

        matches
    }

    /// Scan for an i32 value in a memory range
    pub fn scan_for_i32(&self, target: i32, start_offset: usize, size: usize) -> Vec<usize> {
        let mut matches = Vec::new();

        for offset in (start_offset..start_offset + size).step_by(4) {
            if let Ok(value) = read::<i32>(self.handle, self.base + offset) {
                if value == target {
                    matches.push(offset);
                }
            }
        }

        matches
    }

    /// Scan for a u8 value in a memory range
    pub fn scan_for_u8(&self, target: u8, start_offset: usize, size: usize) -> Vec<usize> {
        let mut matches = Vec::new();

        for offset in start_offset..start_offset + size {
            if let Ok(value) = read::<u8>(self.handle, self.base + offset) {
                if value == target {
                    matches.push(offset);
                }
            }
        }

        matches
    }

    /// Filter previous scan results by new value
    pub fn filter_matches_f32(&self, matches: &[usize], target: f32) -> Vec<usize> {
        let epsilon = 0.01;
        matches
            .iter()
            .filter(|&&offset| {
                if let Ok(value) = read::<f32>(self.handle, self.base + offset) {
                    (value - target).abs() < epsilon
                } else {
                    false
                }
            })
            .copied()
            .collect()
    }

    /// Filter previous scan results by new i32 value
    pub fn filter_matches_i32(&self, matches: &[usize], target: i32) -> Vec<usize> {
        matches
            .iter()
            .filter(|&&offset| {
                if let Ok(value) = read::<i32>(self.handle, self.base + offset) {
                    value == target
                } else {
                    false
                }
            })
            .copied()
            .collect()
    }

    /// Filter previous scan results by new u8 value
    pub fn filter_matches_u8(&self, matches: &[usize], target: u8) -> Vec<usize> {
        matches
            .iter()
            .filter(|&&offset| {
                if let Ok(value) = read::<u8>(self.handle, self.base + offset) {
                    value == target
                } else {
                    false
                }
            })
            .copied()
            .collect()
    }
}

/// Test pointer chains by attempting to follow them
pub struct PointerTester {
    handle: HANDLE,
    base: usize,
}

impl PointerTester {
    pub fn new() -> Result<Self, String> {
        let rb = Process::from_process_name("rekordbox.exe")
            .map_err(|_| "Could not find Rekordbox process!".to_string())?;
        
        let base = rb.get_module_base("rekordbox.exe")
            .map_err(|_| "Could not get module base".to_string())?;

        Ok(PointerTester {
            handle: rb.process_handle,
            base,
        })
    }

    /// Test if a pointer chain is valid (all reads succeed)
    pub fn test_pointer_chain(&self, pointer: &Pointer) -> Result<usize, String> {
        let mut address = self.base;

        for (i, offset) in pointer.offsets.iter().enumerate() {
            address = read::<usize>(self.handle, address + offset)
                .map_err(|_| format!("Failed at offset step {} (offset: {:X})", i, offset))?;
        }
        address += pointer.final_offset;

        Ok(address)
    }

    /// Test a manually constructed pointer chain
    pub fn test_manual_chain(&self, offsets: Vec<usize>, final_offset: usize) -> Result<usize, String> {
        let pointer = Pointer {
            offsets,
            final_offset,
        };
        self.test_pointer_chain(&pointer)
    }

    /// Read value at the end of a pointer chain
    pub fn read_chain_f32(&self, pointer: &Pointer) -> Result<f32, String> {
        let addr = self.test_pointer_chain(pointer)?;
        read::<f32>(self.handle, addr).map_err(|_| "Failed to read f32 value".to_string())
    }

    pub fn read_chain_i32(&self, pointer: &Pointer) -> Result<i32, String> {
        let addr = self.test_pointer_chain(pointer)?;
        read::<i32>(self.handle, addr).map_err(|_| "Failed to read i32 value".to_string())
    }

    pub fn read_chain_u8(&self, pointer: &Pointer) -> Result<u8, String> {
        let addr = self.test_pointer_chain(pointer)?;
        read::<u8>(self.handle, addr).map_err(|_| "Failed to read u8 value".to_string())
    }
}
