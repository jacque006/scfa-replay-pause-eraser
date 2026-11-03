//! Runs the built program the way a player would.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

const SAMPLE: &[u8] = include_bytes!("../data/pause-test.SCFAReplay");

fn tool() -> Command {
    Command::new(env!("CARGO_BIN_EXE_scfa-replay-pause-eraser"))
}

/// A fresh folder holding a copy of the sample as `game.SCFAReplay`.
fn workspace(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "scfa-replay-pause-eraser-{}-{test}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("game.SCFAReplay"), SAMPLE).unwrap();
    dir
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn writes_nopause_file_next_to_input() {
    let dir = workspace("default");
    let out = tool().arg(dir.join("game.SCFAReplay")).output().unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));

    let written = fs::read(dir.join("game.nopause.SCFAReplay")).unwrap();
    assert_eq!(written.len(), SAMPLE.len() - 12 * 3);
    // The input is untouched.
    assert_eq!(fs::read(dir.join("game.SCFAReplay")).unwrap(), SAMPLE);

    let stdout = text(&out.stdout);
    assert!(
        stdout.contains("removed 6 pause requests, 6 resumes"),
        "{stdout}"
    );
    assert!(stdout.contains("--strip-private"), "{stdout}");
}

#[test]
fn does_not_overwrite_unless_forced() {
    let dir = workspace("force");
    let input = dir.join("game.SCFAReplay");
    assert!(tool().arg(&input).output().unwrap().status.success());

    let again = tool().arg(&input).output().unwrap();
    assert!(!again.status.success());
    assert!(text(&again.stderr).contains("already exists"));

    let forced = tool().arg("--force").arg(&input).output().unwrap();
    assert!(forced.status.success(), "{}", text(&forced.stderr));
}

#[test]
fn refuses_to_write_over_the_input() {
    let dir = workspace("same");
    let input = dir.join("game.SCFAReplay");
    let out = tool()
        .args(["--force", "-o"])
        .arg(&input)
        .arg(&input)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert_eq!(fs::read(&input).unwrap(), SAMPLE);
}

#[test]
fn output_name_can_be_chosen() {
    let dir = workspace("output");
    let chosen = dir.join("clean.SCFAReplay");
    let out = tool()
        .arg("-o")
        .arg(&chosen)
        .arg(dir.join("game.SCFAReplay"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(chosen.exists());
    assert!(!dir.join("game.nopause.SCFAReplay").exists());
}

fn pipe(args: &[&str], input: &[u8]) -> Output {
    let mut child = tool()
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn reads_stdin_and_writes_stdout() {
    let dir = workspace("pipe");
    assert!(
        tool()
            .arg(dir.join("game.SCFAReplay"))
            .output()
            .unwrap()
            .status
            .success()
    );
    let from_file = fs::read(dir.join("game.nopause.SCFAReplay")).unwrap();

    let out = pipe(&["-"], SAMPLE);
    assert!(out.status.success(), "{}", text(&out.stderr));
    // Standard output holds only the replay; messages go to standard error.
    assert_eq!(out.stdout, from_file);
    assert!(text(&out.stderr).contains("removed 6 pause requests"));
}

#[test]
fn strip_private_removes_the_player_name() {
    let out = pipe(&["--strip-private", "-"], SAMPLE);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!out.stdout.windows(14).any(|w| w == b"jacque006-loud"));
    assert!(text(&out.stderr).contains("Replaced 1 player name"));
}

#[test]
fn bad_input_writes_nothing() {
    let dir = workspace("bad");
    let input = dir.join("broken.SCFAReplay");
    fs::write(&input, &SAMPLE[..5000]).unwrap();
    let out = tool().arg(&input).output().unwrap();
    assert!(!out.status.success());
    assert!(text(&out.stderr).contains("does not look like a valid replay"));
    assert!(!dir.join("broken.nopause.SCFAReplay").exists());
}

#[test]
fn unknown_options_are_rejected() {
    let out = tool().arg("--bogus").output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = tool().output().unwrap();
    assert_eq!(out.status.code(), Some(2));
}
