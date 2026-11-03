//! Tests the pause removal against the sample replay in `data/`.

use scfa_replay_pause_eraser::replay::{self, Replay};
use scfa_replay_pause_eraser::{Options, REQUEST_PAUSE, RESUME, SINGLE_STEP, erase_pauses};

const SAMPLE: &[u8] = include_bytes!("../data/pause-test.SCFAReplay");
const ADVANCE: u8 = 0;

fn kinds(data: &[u8], replay: &Replay) -> Vec<(u8, Vec<u8>)> {
    replay
        .commands
        .iter()
        .map(|c| (c.kind, data[c.bytes.clone()].to_vec()))
        .collect()
}

/// Total game time in ticks: the sum of every Advance command's tick count.
fn ticks(data: &[u8], replay: &Replay) -> u32 {
    replay
        .commands
        .iter()
        .filter(|c| c.kind == ADVANCE)
        .map(|c| {
            let p = c.bytes.start + 3;
            u32::from_le_bytes(data[p..p + 4].try_into().unwrap())
        })
        .sum()
}

fn name<'a>(data: &'a [u8], range: &std::ops::Range<usize>) -> &'a str {
    std::str::from_utf8(&data[range.clone()]).unwrap()
}

#[test]
fn sample_parses_to_the_end_of_the_file() {
    let r = replay::parse(SAMPLE).unwrap();
    assert_eq!(r.header_end, 10639);
    assert_eq!(r.commands.len(), 701);
    assert_eq!(r.commands.last().unwrap().bytes.end, SAMPLE.len());
    assert_eq!(ticks(SAMPLE, &r), 665);
}

#[test]
fn removes_pauses_and_keeps_everything_else() {
    let (out, report) = erase_pauses(SAMPLE, &Options::default()).unwrap();
    assert_eq!(report.pause_requests_removed, 6);
    assert_eq!(report.resumes_removed, 6);
    assert_eq!(report.resumes_kept, 1);
    assert_eq!(report.player_names, 1);

    let before = replay::parse(SAMPLE).unwrap();
    let after = replay::parse(&out).unwrap();

    // The header is unchanged.
    assert_eq!(&out[..after.header_end], &SAMPLE[..before.header_end]);

    // No pause requests are left, and only the start-of-game resume remains.
    let after_kinds = kinds(&out, &after);
    assert!(!after_kinds.iter().any(|(k, _)| *k == REQUEST_PAUSE));
    assert_eq!(after_kinds.iter().filter(|(k, _)| *k == RESUME).count(), 1);

    // Every other command is identical and in the same order.
    let mut paused = false;
    let expected: Vec<_> = kinds(SAMPLE, &before)
        .into_iter()
        .filter(|(k, _)| match *k {
            REQUEST_PAUSE => {
                paused = true;
                false
            }
            RESUME if paused => {
                paused = false;
                false
            }
            _ => true,
        })
        .collect();
    assert_eq!(after_kinds, expected);

    // Game time is unchanged: 3 bytes were removed per command.
    assert_eq!(ticks(&out, &after), ticks(SAMPLE, &before));
    assert_eq!(out.len(), SAMPLE.len() - 12 * 3);
}

#[test]
fn running_twice_changes_nothing_more() {
    let (once, _) = erase_pauses(SAMPLE, &Options::default()).unwrap();
    let (twice, report) = erase_pauses(&once, &Options::default()).unwrap();
    assert_eq!(once, twice);
    assert_eq!(report.pause_requests_removed, 0);
    assert_eq!(report.resumes_removed, 0);
}

#[test]
fn strip_private_renames_players_only() {
    let options = Options {
        strip_private: true,
        ..Options::default()
    };
    let (out, report) = erase_pauses(SAMPLE, &options).unwrap();
    assert!(report.names_stripped);
    assert_eq!(report.names_left, 0);

    let r = replay::parse(&out).unwrap();
    let sources: Vec<_> = r.sources.iter().map(|s| name(&out, &s.name)).collect();
    assert_eq!(sources, ["Player 1"]);

    let armies: Vec<_> = r
        .armies
        .iter()
        .map(|a| name(&out, a.player_name.as_ref().unwrap()))
        .collect();
    assert_eq!(
        armies,
        ["Player 1", "Doc Tater (AI: LOUD)", "civilian", "civilian"]
    );
    assert!(!out.windows(14).any(|w| w == b"jacque006-loud"));

    // The commander's CustomName is renamed too.
    let custom_names: Vec<_> = r
        .commands
        .iter()
        .filter_map(|c| replay::info_pair(&out, c))
        .filter(|p| &out[p.key.clone()] == b"CustomName")
        .map(|p| name(&out, &p.value))
        .collect();
    assert_eq!(custom_names, ["Player 1"]);

    // Apart from that one command, the commands match the output without
    // --strip-private.
    let (plain, _) = erase_pauses(SAMPLE, &Options::default()).unwrap();
    let plain_r = replay::parse(&plain).unwrap();
    assert_eq!(r.commands.len(), plain_r.commands.len());
    let differing = r
        .commands
        .iter()
        .zip(&plain_r.commands)
        .filter(|(a, b)| out[a.bytes.clone()] != plain[b.bytes.clone()])
        .count();
    assert_eq!(differing, 1);
}

/// The sample's header followed by the given commands.
fn with_commands(commands: &[(u8, &[u8])]) -> Vec<u8> {
    let header_end = replay::parse(SAMPLE).unwrap().header_end;
    let mut data = SAMPLE[..header_end].to_vec();
    for (kind, payload) in commands {
        data.push(*kind);
        data.extend(((payload.len() + 3) as u16).to_le_bytes());
        data.extend_from_slice(payload);
    }
    data
}

#[test]
fn single_steps_are_removed_unless_kept() {
    let one_tick: &[u8] = &1u32.to_le_bytes();
    let data = with_commands(&[
        (ADVANCE, one_tick),
        (REQUEST_PAUSE, &[]),
        (SINGLE_STEP, &[]),
        (ADVANCE, one_tick),
        (RESUME, &[]),
        (ADVANCE, one_tick),
    ]);

    let (out, report) = erase_pauses(&data, &Options::default()).unwrap();
    assert_eq!(report.single_steps_removed, 1);
    let r = replay::parse(&out).unwrap();
    let left: Vec<u8> = r.commands.iter().map(|c| c.kind).collect();
    assert_eq!(left, [ADVANCE, ADVANCE, ADVANCE]);

    let keep = Options {
        keep_single_step: true,
        ..Options::default()
    };
    let (out, report) = erase_pauses(&data, &keep).unwrap();
    assert_eq!(report.single_steps_removed, 0);
    let r = replay::parse(&out).unwrap();
    let left: Vec<u8> = r.commands.iter().map(|c| c.kind).collect();
    assert_eq!(left, [ADVANCE, SINGLE_STEP, ADVANCE, ADVANCE]);
}

#[test]
fn rejects_damaged_files() {
    // Cut off in the middle of a command.
    assert!(erase_pauses(&SAMPLE[..SAMPLE.len() - 1], &Options::default()).is_err());
    // Cut off in the header.
    assert!(erase_pauses(&SAMPLE[..5000], &Options::default()).is_err());
    // Not a replay at all.
    assert!(erase_pauses(b"hello", &Options::default()).is_err());
    assert!(erase_pauses(&[], &Options::default()).is_err());
    // A command that claims to be shorter than its own prefix.
    let bad = with_commands(&[])
        .into_iter()
        .chain([0, 1, 0])
        .collect::<Vec<u8>>();
    assert!(erase_pauses(&bad, &Options::default()).is_err());
}
