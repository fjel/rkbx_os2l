use std::{
    io::Write,
    thread::sleep,
    time,
};

use std::net::TcpStream;
use mdns_sd::{ServiceDaemon, ServiceEvent};

pub struct SoundSwitchConnectionAddr {
    pub soundswitch_ip: String,
    pub soundswitch_port: u16,
}

pub struct SoundSwitchConnector;

impl SoundSwitchConnector {

    pub fn discover_soundswitch() -> SoundSwitchConnectionAddr {
        // Create a daemon
        let mdns = ServiceDaemon::new().expect("Failed to create daemon");
        
        // Browse for a service type.
        let service_type = "_os2l._tcp.local.";
        let receiver = mdns.browse(service_type).expect("Failed to browse");
        
        // Receive the browse events in sync or async. Here is
        // an example of using a thread. Users can call `receiver.recv_async().await`
        // if running in async environment.

        println!("Looking for SoundSwitch application...");
        let connection: SoundSwitchConnectionAddr;
        while let Ok(event) = receiver.recv() {
            match event {
                ServiceEvent::ServiceResolved(info) => {
                    let service_name = info.get_fullname();
                    println!("Resolved a new service: {}", service_name);
                    if service_name.starts_with("SoundSwitch") {
                        println!("Service is SoundSwitch");
                        connection = SoundSwitchConnectionAddr {
                            soundswitch_ip: info.get_addresses_v4().iter().next().unwrap().to_string(),
                            soundswitch_port: info.get_port()
                        };
                        println!("SoundSwitch at {}:{}", connection.soundswitch_ip, connection.soundswitch_port);
                        return connection;
                    }
                }
                _other_event => {
                    // println!("Received other event: {:?}", &other_event);
                    continue;
                }
            }
        };        
        std::process::exit(1);
    }

    pub fn initial_connect(connection: SoundSwitchConnectionAddr) -> TcpStream {
        let mut os2l_stream = TcpStream::connect(format!("{}:{}",connection.soundswitch_ip, connection.soundswitch_port)).unwrap();
        // These values are a guess based on what VirtualDJ sends to SoundSwitch
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_text '%SOUNDSWITCH_ID'\",\"value\":\"\"}\n").unwrap();
        sleep(time::Duration::from_millis(20));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 get_text '%SOUNDSWITCH_ID'\",\"value\":\"\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 get_text '%SOUNDSWITCH_ID'\",\"value\":\"\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 get_text '%SOUNDSWITCH_ID'\",\"value\":\"\"}\n").unwrap();
        sleep(time::Duration::from_millis(30));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 level\",\"value\":1}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 level\",\"value\":1}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 level\",\"value\":1}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 level\",\"value\":1}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"crossfader\",\"value\":0.5}\n").unwrap();
        sleep(time::Duration::from_millis(50));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_bpm\",\"value\":120}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 get_bpm\",\"value\":120}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 get_bpm\",\"value\":120}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 get_bpm\",\"value\":120}\n").unwrap();
        sleep(time::Duration::from_millis(50));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 play\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 play\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 play\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 play\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 loop\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 loop\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 loop\",\"value\":\"off\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 loop\",\"value\":\"off\"}\n").unwrap();
        sleep(time::Duration::from_millis(50));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_loop\",\"value\":8}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 get_loop\",\"value\":16}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 get_loop\",\"value\":8}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 get_loop\",\"value\":8}\n").unwrap();
        sleep(time::Duration::from_millis(50));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 loop_roll 0.03125 ? constant 0.03125 : deck 1 loop_roll 0.0625 ? constant 0.0625 : deck 1 loop_roll 0.125 ? constant 0.125 : deck 1 loop_roll 0.25 ? constant 0.25 : deck 1 loop_roll 0.5 ? constant 0.5 : deck 1 loop_roll 0.75 ? constant 0.75 : deck 1 loop_roll 1 ? constant 1 : deck 1 loop_roll 2 ? constant 2 : deck 1 loop_roll 4 ? constant 4 : constant 0\",\"value\":0}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 2 loop_roll 0.03125 ? constant 0.03125 : deck 2 loop_roll 0.0625 ? constant 0.0625 : deck 2 loop_roll 0.125 ? constant 0.125 : deck 2 loop_roll 0.25 ? constant 0.25 : deck 2 loop_roll 0.5 ? constant 0.5 : deck 2 loop_roll 0.75 ? constant 0.75 : deck 2 loop_roll 1 ? constant 1 : deck 2 loop_roll 2 ? constant 2 : deck 2 loop_roll 4 ? constant 4 : constant 0\",\"value\":0}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 loop_roll 0.03125 ? constant 0.03125 : deck 3 loop_roll 0.0625 ? constant 0.0625 : deck 3 loop_roll 0.125 ? constant 0.125 : deck 3 loop_roll 0.25 ? constant 0.25 : deck 3 loop_roll 0.5 ? constant 0.5 : deck 3 loop_roll 0.75 ? constant 0.75 : deck 3 loop_roll 1 ? constant 1 : deck 3 loop_roll 2 ? constant 2 : deck 3 loop_roll 4 ? constant 4 : constant 0\",\"value\":0}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 loop_roll 0.03125 ? constant 0.03125 : deck 4 loop_roll 0.0625 ? constant 0.0625 : deck 4 loop_roll 0.125 ? constant 0.125 : deck 4 loop_roll 0.25 ? constant 0.25 : deck 4 loop_roll 0.5 ? constant 0.5 : deck 4 loop_roll 0.75 ? constant 0.75 : deck 4 loop_roll 1 ? constant 1 : deck 4 loop_roll 2 ? constant 2 : deck 4 loop_roll 4 ? constant 4 : constant 0\",\"value\":0}\n").unwrap();
        sleep(time::Duration::from_millis(50));
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_text '%SOUNDSWITCH_ID'\",\"value\":\"{00000000-0000-0000-0000-000000000000}\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 3 get_text '%SOUNDSWITCH_ID'\",\"value\":\"{00000000-0000-0000-0000-000000000000}\"}\n").unwrap();
        os2l_stream.write(b"{\"evt\":\"subscribed\",\"trigger\":\"deck 4 get_text '%SOUNDSWITCH_ID'\",\"value\":\"{00000000-0000-0000-0000-000000000000}\"}\n").unwrap();
        sleep(time::Duration::from_millis(50));

        return os2l_stream;
    }

    pub fn send_beatpos(os2l_stream: &mut TcpStream, last_beat: i32) {
        os2l_stream.write(format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_beatpos\",\"value\":{}}}\n", last_beat).as_str().as_bytes()).unwrap();
    }

    pub fn send_beat(os2l_stream: &mut TcpStream, last_beat: i32, last_bpm: f32) {
        os2l_stream.write(format!("{{\"evt\":\"beat\",\"change\":false,\"pos\":{},\"bpm\":{},\"strength\":0}}", last_beat, last_bpm).as_str().as_bytes()).unwrap();
    }

    pub fn send_track(os2l_stream: &mut TcpStream, last_master_path: &mut String) {
        let master_path = last_master_path.replace("/", "\\\\"); // Replace slashes with backslashes for serialization
        os2l_stream.write(format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_filepath\",\"value\":\"{}\"}}\n", master_path).as_str().as_bytes()).unwrap();
    }

    pub fn send_time(os2l_stream: &mut TcpStream, last_time: i32) {
        os2l_stream.write(format!("{{\"evt\":\"subscribed\",\"trigger\":\"deck 1 get_time elapsed absolute\",\"value\":{}}}\n", last_time).as_str().as_bytes()).unwrap();
    }
    
}


