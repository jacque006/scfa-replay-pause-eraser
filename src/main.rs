#![forbid(unsafe_code)]

use scfa_replay_pause_eraser::{Options, Report, erase_pauses, placeholder_name};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
Removes pauses from a Supreme Commander: Forged Alliance replay.

Usage: scfa-replay-pause-eraser [options] <replay>

  <replay>              the .SCFAReplay file to read, or - to read standard input

Options:
  -o, --output <file>   where to write the result, or - for standard output.
                        Default: <replay name>.nopause.SCFAReplay next to the input,
                        or standard output when reading standard input.
      --keep-single-step
                        keep single-step commands (stepping one tick while paused).
                        By default they are removed along with the pauses.
      --strip-private   replace player and observer names with \"Player 1\", \"Player 2\", ...
  -f, --force           overwrite the output file if it already exists
  -h, --help            show this help
  -V, --version         show the version

The input file is never changed.";

struct Args {
    input: String,
    output: Option<String>,
    force: bool,
    options: Options,
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(Some(args)) => args,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Returns `None` when help or the version was printed.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Option<Args>, String> {
    let mut input = None;
    let mut output = None;
    let mut force = false;
    let mut options = Options::default();

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("scfa-replay-pause-eraser {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-o" | "--output" => {
                output = Some(args.next().ok_or("--output needs a file name")?);
            }
            "-f" | "--force" => force = true,
            "--keep-single-step" => options.keep_single_step = true,
            "--strip-private" => options.strip_private = true,
            _ if arg.starts_with('-') && arg != "-" => {
                return Err(format!("unknown option {arg}"));
            }
            _ if input.is_some() => return Err("only one replay can be given".to_string()),
            _ => input = Some(arg),
        }
    }

    let input = input.ok_or("no replay given")?;
    Ok(Some(Args {
        input,
        output,
        force,
        options,
    }))
}

fn run(args: &Args) -> Result<(), String> {
    let data = if args.input == "-" {
        let mut data = Vec::new();
        io::stdin()
            .read_to_end(&mut data)
            .map_err(|e| format!("could not read standard input: {e}"))?;
        data
    } else {
        fs::read(&args.input).map_err(|e| format!("could not read {}: {e}", args.input))?
    };

    let (out, report) = erase_pauses(&data, &args.options)
        .map_err(|e| format!("{} does not look like a valid replay: {e}", args.input))?;

    let output = match (&args.output, args.input.as_str()) {
        (Some(output), _) => output.clone(),
        (None, "-") => "-".to_string(),
        (None, input) => default_output(Path::new(input)).display().to_string(),
    };

    // Status messages go to standard output, unless the replay itself is
    // being written there.
    let to_stdout = output == "-";
    if to_stdout {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(&out)
            .and_then(|()| stdout.flush())
            .map_err(|e| format!("could not write standard output: {e}"))?;
    } else {
        write_file(&args.input, &output, &out, args.force)?;
    }

    for line in summary(&output, &report) {
        if to_stdout {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    Ok(())
}

/// `games/x.SCFAReplay` becomes `games/x.nopause.SCFAReplay`.
fn default_output(input: &Path) -> PathBuf {
    let mut name = OsString::from(input.file_stem().unwrap_or_default());
    name.push(".nopause.SCFAReplay");
    input.with_file_name(name)
}

fn write_file(input: &str, output: &str, data: &[u8], force: bool) -> Result<(), String> {
    if input != "-" && same_file(Path::new(input), Path::new(output)) {
        return Err("the output file is the input file; choose a different --output".to_string());
    }
    let mut open = OpenOptions::new();
    if force {
        open.write(true).create(true).truncate(true);
    } else {
        open.write(true).create_new(true);
    }
    let mut file = open.open(output).map_err(|e| {
        if e.kind() == io::ErrorKind::AlreadyExists {
            format!("{output} already exists; use --force to overwrite it")
        } else {
            format!("could not create {output}: {e}")
        }
    })?;
    file.write_all(data)
        .map_err(|e| format!("could not write {output}: {e}"))
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

fn summary(output: &str, report: &Report) -> Vec<String> {
    let destination = if output == "-" {
        "standard output"
    } else {
        output
    };
    let mut lines = vec![format!(
        "Wrote {destination}: removed {}, {}, and {}.",
        count(report.pause_requests_removed, "pause request"),
        count(report.resumes_removed, "resume"),
        count(report.single_steps_removed, "single step"),
    )];
    if report.resumes_kept > 0 {
        lines.push(format!(
            "Kept {} with no pause before it (likely the game starting).",
            count(report.resumes_kept, "resume"),
        ));
    }
    let names = count(report.player_names, "player name");
    if report.names_stripped {
        lines.push(format!(
            "Replaced {names}, using \"{}\" onward.",
            placeholder_name(0)
        ));
        if report.names_left > 0 {
            lines.push(format!(
                "Warning: {} of the original player names can still be found \
                 in the file. Do not share it without checking.",
                report.names_left,
            ));
        }
    } else if report.player_names > 0 {
        lines.push(format!(
            "Privacy: this replay still contains {names}. To replace them, \
             run again with --strip-private. Otherwise, only share it if you are \
             fine with those names being seen."
        ));
    }
    lines
}

/// `count(1, "resume")` is "1 resume"; `count(2, "resume")` is "2 resumes".
fn count(n: usize, thing: &str) -> String {
    if n == 1 {
        format!("1 {thing}")
    } else {
        format!("{n} {thing}s")
    }
}
