use faf_replay_parser::iter::prelude::*;
use faf_replay_parser::parser::parse_header;
use faf_replay_parser::replay::SimData;
use faf_replay_parser::scfa::ReplayCommand as SCFACommand;
use faf_replay_parser::version::Command;
use faf_replay_parser::SCFA;
use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::time::Duration;

fn main() {
    // Read replay file
    let mut f = BufReader::new(File::open("./data/pause-test.SCFAReplay").expect("File should open"));
    let mut data = Vec::new();
    f.read_to_end(&mut data).expect("File should be readable");
    let mut cur = Cursor::new(&data);

    let _header = parse_header(&mut cur).expect("Replay header should be valid");
    let body_start = cur.position() as usize;
    let body_data = &data[body_start..];

    let mut sim = SimData::new();

    let _ = body_data
        .iter_commands::<SCFA>()
        .map(|cmd| cmd.expect("Command data should be valid"))
        .inspect(|cmd| {
            // Ignore errors here to fully process desynced replays
            SCFACommand::process_command(&mut sim, &cmd).unwrap_or(())
        })
        .count();

    println!(
        "Game time: {:?}",
        Duration::from_millis(sim.tick as u64 * 100)
    );
    if !sim.desync_ticks.is_none() {
        println!("Replay desynced!");
    }
}

