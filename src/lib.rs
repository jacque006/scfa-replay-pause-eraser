//! Removes pause commands from Supreme Commander: Forged Alliance replays.
//!
//! A replay is a header followed by a list of commands. Pausing and resuming
//! the game are recorded as their own tiny commands that carry no data and do
//! not change the game simulation. This tool copies the replay unchanged,
//! except that it leaves each pause, and the resume that ends it, out.
//! See [`erase_pauses`].
//!
//! Separately, [`strip_private`] can replace player names, for sharing.

#![forbid(unsafe_code)]

pub mod replay;

pub use replay::ParseError;
use std::ops::Range;

/// A player asked to pause the game.
pub const REQUEST_PAUSE: u8 = 4;
/// A player resumed the game.
pub const RESUME: u8 = 5;
/// While paused, a player advanced the game by one tick.
pub const SINGLE_STEP: u8 = 6;

#[derive(Default)]
pub struct Options {
    /// Keep `SingleStep` commands instead of removing them.
    pub keep_single_step: bool,
    /// Also replace player names. See [`strip_private`].
    pub strip_private: bool,
}

/// What [`erase_pauses`] did.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub pause_requests_removed: usize,
    pub resumes_removed: usize,
    pub single_steps_removed: usize,
    /// Resumes with no pause before them. These are kept: the sample replay
    /// has one at the very start, likely the game starting, and removing it
    /// could stop the replay from playing.
    pub resumes_kept: usize,
    /// How many human player and observer names the replay contains.
    pub player_names: usize,
    /// Whether those names were replaced.
    pub names_stripped: bool,
    /// After replacing, how many of the original names could still be found
    /// somewhere in the file. Should always be 0.
    pub names_left: usize,
}

/// Returns a copy of `data` with the pause commands left out.
///
/// Every pause request is removed, along with the resume that ends it.
/// A resume that does not follow a pause request is kept. Single steps are
/// removed unless `keep_single_step` is set. The header and every other
/// command are copied byte for byte.
///
/// The result is parsed again before it is returned, so a malformed output
/// is reported as an error instead of being written.
pub fn erase_pauses(data: &[u8], options: &Options) -> Result<(Vec<u8>, Report), ParseError> {
    let replay = replay::parse(data)?;
    let mut report = Report {
        player_names: replay.sources.len(),
        ..Report::default()
    };

    let mut out = data[..replay.header_end].to_vec();

    let mut paused = false;
    for command in &replay.commands {
        let remove = match command.kind {
            REQUEST_PAUSE => {
                report.pause_requests_removed += 1;
                paused = true;
                true
            }
            RESUME if paused => {
                report.resumes_removed += 1;
                paused = false;
                true
            }
            RESUME => {
                report.resumes_kept += 1;
                false
            }
            SINGLE_STEP if !options.keep_single_step => {
                report.single_steps_removed += 1;
                true
            }
            _ => false,
        };
        if !remove {
            out.extend_from_slice(&data[command.bytes.clone()]);
        }
    }

    if options.strip_private {
        let (stripped, names_left) = strip_private(&out)?;
        out = stripped;
        report.names_stripped = true;
        report.names_left = names_left;
    }

    replay::parse(&out)?;
    Ok((out, report))
}

/// The name that replaces the name of command source `index`.
pub fn placeholder_name(index: usize) -> String {
    format!("Player {}", index + 1)
}

/// Returns a copy of `data` with each human player and observer name replaced
/// by "Player 1", "Player 2", and so on. AI and civilian names are kept.
///
/// A name can appear in three places, and each is replaced:
/// - the header's list of command sources,
/// - the `PlayerName` in that player's army settings,
/// - a `ProcessInfoPair` command that sets a `CustomName` equal to the name.
///
/// When a replacement changes the length of something with a size prefix,
/// that size is rewritten too. Also returns how many of the original names
/// can still be found in the result, as a final check.
pub fn strip_private(data: &[u8]) -> Result<(Vec<u8>, usize), ParseError> {
    let replay = replay::parse(data)?;
    let names: Vec<&[u8]> = replay
        .sources
        .iter()
        .map(|s| &data[s.name.clone()])
        .collect();

    // (range in `data`, bytes to put there instead)
    let mut edits: Vec<(Range<usize>, Vec<u8>)> = Vec::new();

    for (index, source) in replay.sources.iter().enumerate() {
        edits.push((source.name.clone(), placeholder_name(index).into_bytes()));
    }

    for army in &replay.armies {
        if let (Some(index), Some(name)) = (army.source, army.player_name.clone()) {
            let new_name = placeholder_name(index).into_bytes();
            let new_size = army.blob.len() - name.len() + new_name.len();
            let Ok(new_size) = u32::try_from(new_size) else {
                continue;
            };
            edits.push((army.size_field.clone(), new_size.to_le_bytes().to_vec()));
            edits.push((name, new_name));
        }
    }

    for command in &replay.commands {
        let Some(pair) = replay::info_pair(data, command) else {
            continue;
        };
        if &data[pair.key.clone()] != b"CustomName" {
            continue;
        }
        let value = &data[pair.value.clone()];
        let Some(index) = names.iter().position(|name| *name == value) else {
            continue;
        };
        let new_name = placeholder_name(index).into_bytes();
        let new_length = command.bytes.len() - value.len() + new_name.len();
        let Ok(new_length) = u16::try_from(new_length) else {
            continue;
        };
        let length_field = command.bytes.start + 1..command.bytes.start + 3;
        edits.push((length_field, new_length.to_le_bytes().to_vec()));
        edits.push((pair.value, new_name));
    }

    let out = apply_edits(data, edits);
    let names_left = names
        .iter()
        .filter(|name| contains_cstring(&out, name))
        .count();
    Ok((out, names_left))
}

/// Copies `data`, putting each edit's bytes in place of its range.
/// The ranges must not overlap.
fn apply_edits(data: &[u8], mut edits: Vec<(Range<usize>, Vec<u8>)>) -> Vec<u8> {
    edits.sort_by_key(|(range, _)| range.start);
    let mut out = Vec::with_capacity(data.len());
    let mut copied_up_to = 0;
    for (range, replacement) in edits {
        out.extend_from_slice(&data[copied_up_to..range.start]);
        out.extend_from_slice(&replacement);
        copied_up_to = range.end;
    }
    out.extend_from_slice(&data[copied_up_to..]);
    out
}

/// Whether `text`, followed by a zero byte, appears anywhere in `data`.
/// Replay strings end with a zero byte, so this finds any leftover copy.
fn contains_cstring(data: &[u8], text: &[u8]) -> bool {
    !text.is_empty()
        && data
            .windows(text.len() + 1)
            .any(|w| &w[..text.len()] == text && w[text.len()] == 0)
}
