use std::io::{self, Write};
use std::path::Path;
use toy_arms::external::{read, Process};
use winapi::um::winnt::HANDLE;
#[allow(unused_imports)]
use std::marker::PhantomData;

// Include the modules from the main crate
include!("../offsets.rs");
include!("../offset_tools.rs");

fn main() {
    println!("=== Rekordbox Offset Helper Tool ===\n");

    if !Path::new("./offsets").exists() {
        println!("❌ Offsets file not found! Please ensure 'offsets' file exists.");
        return;
    }

    loop {
        println!("\nSelect a tool:");
        println!("1. Validate existing offsets");
        println!("2. Memory scanner (search for values)");
        println!("3. Test pointer chain");
        println!("4. Exit");
        print!("\nChoice: ");
        io::stdout().flush().unwrap();

        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();

        match input.trim() {
            "1" => validate_offsets(),
            "2" => memory_scanner_menu(),
            "3" => test_pointer_chain(),
            "4" => break,
            _ => println!("Invalid choice!"),
        }
    }
}

fn validate_offsets() {
    println!("\n=== Offset Validator ===");
    
    let validator = match OffsetValidator::new() {
        Ok(v) => v,
        Err(e) => {
            println!("❌ Error: {}", e);
            return;
        }
    };

    let version_offsets = RekordboxOffsets::from_file("offsets");
    let mut versions: Vec<String> = version_offsets.keys().map(|x| x.to_string()).collect();
    versions.sort();
    versions.reverse();

    println!("\nAvailable versions: {}", versions.join(", "));
    print!("Enter version to validate (or press Enter for latest): ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    let version = input.trim();
    
    let target_version = if version.is_empty() {
        &versions[0]
    } else {
        version
    };

    let offsets = match version_offsets.get(target_version) {
        Some(o) => o,
        None => {
            println!("❌ Version not found!");
            return;
        }
    };

    println!("\n📋 Validating offsets for Rekordbox {}...\n", target_version);
    let results = validator.validate_offsets(&offsets);

    let mut all_valid = true;
    for result in &results {
        let status = if result.valid { "✅" } else { "❌" };
        println!("{} {:20} = {:15} ({})", status, result.name, result.value, result.reason);
        if !result.valid {
            all_valid = false;
        }
    }

    println!();
    if all_valid {
        println!("🎉 All offsets validated successfully!");
    } else {
        println!("⚠️  Some offsets failed validation. Please check the values above.");
    }
}

fn memory_scanner_menu() {
    println!("\n=== Memory Scanner ===");
    
    let scanner = match MemoryScanner::new() {
        Ok(s) => s,
        Err(e) => {
            println!("❌ Error: {}", e);
            return;
        }
    };

    println!("\nSelect data type to search for:");
    println!("1. Float (f32) - for BPM");
    println!("2. Integer (i32) - for bars, beats, track IDs, time");
    println!("3. Byte (u8) - for deck index");
    print!("\nChoice: ");
    io::stdout().flush().unwrap();

    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();

    match input.trim() {
        "1" => scan_float(&scanner),
        "2" => scan_i32(&scanner),
        "3" => scan_u8(&scanner),
        _ => println!("Invalid choice!"),
    }
}

fn scan_float(scanner: &MemoryScanner) {
    println!("\n🔍 Float Scanner");
    println!("💡 Tip: Search for BPM value (e.g., 128.00)");
    
    print!("\nEnter value to search for: ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    
    let target: f32 = match input.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            println!("❌ Invalid float value!");
            return;
        }
    };

    println!("\nSearching... (this may take a while)");
    
    // Search in a reasonable range (first 100MB of module)
    let matches = scanner.scan_for_float(target, 0, 100_000_000);
    
    println!("\n✅ Found {} matches:", matches.len());
    
    if matches.len() > 100 {
        println!("(Showing first 100 results)");
        for offset in matches.iter().take(100) {
            println!("  Offset: 0x{:08X}", offset);
        }
    } else {
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }
    }

    if matches.len() > 1 && matches.len() < 10000 {
        println!("\n💡 Too many results? Change the value in Rekordbox and scan again to filter.");
        print!("Press Enter to filter, or type 'q' to quit: ");
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() != "q" {
            filter_scan_float(scanner, matches);
        }
    }
}

fn filter_scan_float(scanner: &MemoryScanner, mut matches: Vec<usize>) {
    loop {
        print!("\nEnter new value (or 'q' to quit): ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() == "q" {
            break;
        }
        
        let target: f32 = match input.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                println!("❌ Invalid float value!");
                continue;
            }
        };

        matches = scanner.filter_matches_f32(&matches, target);
        
        println!("✅ {} matches remaining:", matches.len());
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }

        if matches.len() <= 5 {
            println!("\n🎉 Found {} candidate(s)! Use Cheat Engine pointer scan on these addresses.", matches.len());
            break;
        }
    }
}

fn scan_i32(scanner: &MemoryScanner) {
    println!("\n🔍 Integer Scanner");
    println!("💡 Tip: Search for bar number, beat number, track ID, or time in milliseconds");
    
    print!("\nEnter value to search for: ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    
    let target: i32 = match input.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            println!("❌ Invalid integer value!");
            return;
        }
    };

    println!("\nSearching... (this may take a while)");
    
    let matches = scanner.scan_for_i32(target, 0, 100_000_000);
    
    println!("\n✅ Found {} matches:", matches.len());
    
    if matches.len() > 100 {
        println!("(Showing first 100 results)");
        for offset in matches.iter().take(100) {
            println!("  Offset: 0x{:08X}", offset);
        }
    } else {
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }
    }

    if matches.len() > 1 && matches.len() < 10000 {
        println!("\n💡 Too many results? Change the value in Rekordbox and scan again to filter.");
        print!("Press Enter to filter, or type 'q' to quit: ");
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() != "q" {
            filter_scan_i32(scanner, matches);
        }
    }
}

fn filter_scan_i32(scanner: &MemoryScanner, mut matches: Vec<usize>) {
    loop {
        print!("\nEnter new value (or 'q' to quit): ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() == "q" {
            break;
        }
        
        let target: i32 = match input.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                println!("❌ Invalid integer value!");
                continue;
            }
        };

        matches = scanner.filter_matches_i32(&matches, target);
        
        println!("✅ {} matches remaining:", matches.len());
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }

        if matches.len() <= 5 {
            println!("\n🎉 Found {} candidate(s)! Use Cheat Engine pointer scan on these addresses.", matches.len());
            break;
        }
    }
}

fn scan_u8(scanner: &MemoryScanner) {
    println!("\n🔍 Byte Scanner");
    println!("💡 Tip: Search for masterdeck index (0 or 1)");
    
    print!("\nEnter value to search for (0-255): ");
    io::stdout().flush().unwrap();
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    
    let target: u8 = match input.trim().parse() {
        Ok(v) => v,
        Err(_) => {
            println!("❌ Invalid byte value!");
            return;
        }
    };

    println!("\nSearching... (this may take a while)");
    
    let matches = scanner.scan_for_u8(target, 0, 100_000_000);
    
    println!("\n✅ Found {} matches:", matches.len());
    
    if matches.len() > 100 {
        println!("(Showing first 100 results)");
        for offset in matches.iter().take(100) {
            println!("  Offset: 0x{:08X}", offset);
        }
    } else {
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }
    }

    if matches.len() > 1 && matches.len() < 10000 {
        println!("\n💡 Too many results? Change the value in Rekordbox and scan again to filter.");
        print!("Press Enter to filter, or type 'q' to quit: ");
        io::stdout().flush().unwrap();
        
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() != "q" {
            filter_scan_u8(scanner, matches);
        }
    }
}

fn filter_scan_u8(scanner: &MemoryScanner, mut matches: Vec<usize>) {
    loop {
        print!("\nEnter new value (or 'q' to quit): ");
        io::stdout().flush().unwrap();
        let mut input = String::new();
        io::stdin().read_line(&mut input).unwrap();
        
        if input.trim().to_lowercase() == "q" {
            break;
        }
        
        let target: u8 = match input.trim().parse() {
            Ok(v) => v,
            Err(_) => {
                println!("❌ Invalid byte value!");
                continue;
            }
        };

        matches = scanner.filter_matches_u8(&matches, target);
        
        println!("✅ {} matches remaining:", matches.len());
        for offset in &matches {
            println!("  Offset: 0x{:08X}", offset);
        }

        if matches.len() <= 5 {
            println!("\n🎉 Found {} candidate(s)! Use Cheat Engine pointer scan on these addresses.", matches.len());
            break;
        }
    }
}

fn test_pointer_chain() {
    println!("\n=== Pointer Chain Tester ===");
    
    let tester = match PointerTester::new() {
        Ok(t) => t,
        Err(e) => {
            println!("❌ Error: {}", e);
            return;
        }
    };

    println!("\nEnter pointer chain (space-separated hex values, e.g., '05B75C38 8 58 1CD8'):");
    print!("> ");
    io::stdout().flush().unwrap();
    
    let mut input = String::new();
    io::stdin().read_line(&mut input).unwrap();
    
    let parts: Vec<&str> = input.trim().split_whitespace().collect();
    
    if parts.is_empty() {
        println!("❌ No offsets provided!");
        return;
    }

    let mut offsets = Vec::new();
    for part in &parts[..parts.len()-1] {
        match usize::from_str_radix(part, 16) {
            Ok(v) => offsets.push(v),
            Err(_) => {
                println!("❌ Invalid hex value: {}", part);
                return;
            }
        }
    }

    let final_offset = match usize::from_str_radix(parts[parts.len()-1], 16) {
        Ok(v) => v,
        Err(_) => {
            println!("❌ Invalid hex value: {}", parts[parts.len()-1]);
            return;
        }
    };

    println!("\nTesting pointer chain...");
    match tester.test_manual_chain(offsets.clone(), final_offset) {
        Ok(addr) => {
            println!("✅ Pointer chain is valid!");
            println!("   Final address: 0x{:08X}", addr);
            
            println!("\nWhat data type should we read?");
            println!("1. Float (f32)");
            println!("2. Integer (i32)");
            println!("3. Byte (u8)");
            print!("\nChoice: ");
            io::stdout().flush().unwrap();
            
            let mut choice = String::new();
            io::stdin().read_line(&mut choice).unwrap();
            
            let pointer = Pointer { offsets, final_offset };
            
            match choice.trim() {
                "1" => {
                    match tester.read_chain_f32(&pointer) {
                        Ok(val) => println!("   Value: {}", val),
                        Err(e) => println!("❌ Failed to read: {}", e),
                    }
                }
                "2" => {
                    match tester.read_chain_i32(&pointer) {
                        Ok(val) => println!("   Value: {}", val),
                        Err(e) => println!("❌ Failed to read: {}", e),
                    }
                }
                "3" => {
                    match tester.read_chain_u8(&pointer) {
                        Ok(val) => println!("   Value: {}", val),
                        Err(e) => println!("❌ Failed to read: {}", e),
                    }
                }
                _ => println!("Invalid choice!"),
            }
        }
        Err(e) => {
            println!("❌ Pointer chain is invalid!");
            println!("   Error: {}", e);
        }
    }
}
