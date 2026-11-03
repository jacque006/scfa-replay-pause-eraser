# scfa-replay-pause-eraser
Rust tool to remove game pauses from Supreme Commander: Forged Alliance (SCFA) replays, including replays from the [LOUD mod](https://www.moddb.com/mods/loud-ai-supreme-commander-forged-alliance).

## motivation
The author plays solo SCFA games using the [LOUD mod](https://www.moddb.com/mods/loud-ai-supreme-commander-forged-alliance) with frequent pausing to evaluate the state of the game & queue up orders. During the viewing of replays the pause/unpause dialog will pop up frequently which is very annoying. This tool seeks to strip those pause/unpause events from the replay file.

## usage
```
scfa-replay-pause-eraser [options] <replay>
```

Remove the pauses from a replay. This writes `my-game.nopause.SCFAReplay` next to the original, which is never changed:
```
scfa-replay-pause-eraser my-game.SCFAReplay
```

Options:
* `-o, --output <file>` choose the output file name. Use `-` to write to standard output.
* `--strip-private` replace player and observer names with `Player 1`, `Player 2`, ... before sharing a replay. AI and civilian names are kept.
* `--keep-single-step` keep single-step commands (stepping the game one tick while paused). By default they are removed along with the pauses.
* `-f, --force` overwrite the output file if it already exists. Without it, the tool refuses.
* `-h, --help`, `-V, --version`

Use `-` as the replay to read from standard input. The cleaned replay then goes to standard output, and messages go to standard error, so it can be used in pipes:
```
cat my-game.SCFAReplay | scfa-replay-pause-eraser - > clean.SCFAReplay
```

Clean every replay in a folder:
```
for f in *.SCFAReplay; do scfa-replay-pause-eraser "$f"; done
```

The tool exits with `0` on success, `1` if a replay could not be read or written, and `2` for a mistake in the options. If a replay looks damaged, nothing is written.

### privacy
Replays contain the names of the human players and observers. After writing a replay, the tool says how many it contains. Re-run with `--strip-private` to replace them before sharing, or share the replay as-is if you accept that.

## how it works
A replay is a header (game settings, mods, players) followed by a list of commands, one after another. Each command starts with a 1-byte type and a 2-byte length.

Pausing (`RequestPause`, type 4) and resuming (`Resume`, type 5) are their own 3-byte commands. They carry no data and do not change the game itself, so removing them does not change what happens in the replay. The tool copies the header and every command byte for byte, except:

* each `RequestPause`,
* the `Resume` that ends each pause. A `Resume` with no pause before it is kept. One appears at the very start of a game, and is likely the game starting.
* each `SingleStep` (type 6), unless `--keep-single-step` is given.

The output is then read back in, to check it is a well-formed replay, before it is written.

All of the logic is in [`src/lib.rs`](src/lib.rs), in `erase_pauses`. The code that finds where things are in a replay file is in [`src/replay.rs`](src/replay.rs). The tool uses no third-party code.

With `--strip-private`, `strip_private` in the same file replaces each player name where it appears: the header's player list, that player's army settings, and the `CustomName` the game gives that player's commander.

## building
```
cargo build --release
cargo test
```
