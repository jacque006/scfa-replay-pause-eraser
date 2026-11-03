//! A small reader for the `.SCFAReplay` file layout.
//!
//! This module only finds *where things are* in a replay. It never changes any
//! bytes. It is deliberately separate from the pause-removal logic in `lib.rs`
//! so it can later be replaced by the `faf-replay-parser` crate.
//!
//! Layout (all integers are little-endian):
//!
//! ```text
//! header
//!   cstring      game version, e.g. "Supreme Commander v1.60.   6"
//!   3 bytes      separator
//!   cstring      "Replay v1.9\r\n<map path>"
//!   4 bytes      separator
//!   u32 + blob   mods (a serialized Lua table)
//!   u32 + blob   scenario (a serialized Lua table)
//!   u8           number of command sources (players and observers); for each:
//!                  cstring  player name
//!                  u32      player id
//!   u8           cheats enabled
//!   u8           number of armies; for each:
//!                  u32 + blob  army settings (a serialized Lua table)
//!                  u8          index of the army's command source, or 255 for none
//!                  u8          present only when the index is not 255
//!   u32          random seed
//! body
//!   commands until the end of the file; each:
//!     u8    command type
//!     u16   length of the whole command in bytes, including these 3 bytes
//!     ...   payload
//! ```

use std::fmt;
use std::ops::Range;

/// The highest command type the SCFA engine writes (`EndGame`).
const MAX_COMMAND_TYPE: u8 = 23;

/// Byte ranges of the parts of a replay that this tool cares about.
pub struct Replay {
    /// Where the header ends and the command stream begins.
    pub header_end: usize,
    pub sources: Vec<Source>,
    pub armies: Vec<Army>,
    pub commands: Vec<Command>,
}

/// A command source: a human player or an observer.
pub struct Source {
    /// The player name, without its terminating zero byte.
    pub name: Range<usize>,
}

/// One army's settings from the header.
pub struct Army {
    /// The u32 that holds the size of the settings blob.
    pub size_field: Range<usize>,
    /// The serialized Lua table with the army's settings.
    pub blob: Range<usize>,
    /// Index into `sources`, or `None` for AI and civilian armies.
    pub source: Option<usize>,
    /// The `PlayerName` string inside the blob, without its zero byte.
    pub player_name: Option<Range<usize>>,
}

/// One command in the body.
pub struct Command {
    pub kind: u8,
    /// The whole command, including its 3-byte type and length prefix.
    pub bytes: Range<usize>,
}

#[derive(Debug)]
pub struct ParseError {
    pub offset: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at byte {})", self.message, self.offset)
    }
}

impl std::error::Error for ParseError {}

type Result<T> = std::result::Result<T, ParseError>;

/// Reads a whole replay and checks that every part fits together exactly:
/// every size prefix matches its contents, and the commands end exactly at
/// the end of the file.
pub fn parse(data: &[u8]) -> Result<Replay> {
    let mut r = Reader::new(data, 0, data.len());

    let version = r.cstring()?;
    if !data[version].starts_with(b"Supreme Commander") {
        return Err(r.error_at(0, "not a Supreme Commander replay"));
    }
    r.take(3)?;
    r.cstring()?;
    r.take(4)?;

    let mods_size = r.u32()? as usize;
    r.take(mods_size)?;
    let scenario_size = r.u32()? as usize;
    r.take(scenario_size)?;

    let mut sources = Vec::new();
    for _ in 0..r.u8()? {
        let name = r.cstring()?;
        r.u32()?;
        sources.push(Source { name });
    }

    r.u8()?; // cheats enabled

    let mut armies = Vec::new();
    for _ in 0..r.u8()? {
        let size_start = r.pos;
        let size = r.u32()? as usize;
        let size_field = size_start..r.pos;
        let blob = r.take(size)?;
        let player_name = find_player_name(data, blob.clone())?;
        let source = match r.u8()? {
            255 => None,
            index => {
                r.u8()?;
                Some(index as usize)
            }
        };
        armies.push(Army {
            size_field,
            blob,
            source,
            player_name,
        });
    }

    r.u32()?; // random seed
    let header_end = r.pos;

    let mut commands = Vec::new();
    while r.pos < data.len() {
        let start = r.pos;
        let kind = r.u8()?;
        if kind > MAX_COMMAND_TYPE {
            return Err(r.error_at(start, &format!("unknown command type {kind}")));
        }
        let length = r.u16()? as usize;
        if length < 3 {
            return Err(r.error_at(start, "command length is too short"));
        }
        r.take(length - 3)?;
        commands.push(Command {
            kind,
            bytes: start..r.pos,
        });
    }

    Ok(Replay {
        header_end,
        sources,
        armies,
        commands,
    })
}

/// The command type that sets a named value on an entity, e.g. the
/// `CustomName` of a player's commander.
pub const PROCESS_INFO_PAIR: u8 = 11;

/// The parts of a `ProcessInfoPair` command. Its payload is:
/// u32 entity id, cstring key, cstring value.
pub struct InfoPair {
    pub key: Range<usize>,
    pub value: Range<usize>,
}

/// Reads a `ProcessInfoPair` command, or returns `None` for any other command.
pub fn info_pair(data: &[u8], command: &Command) -> Option<InfoPair> {
    if command.kind != PROCESS_INFO_PAIR {
        return None;
    }
    let mut r = Reader::new(data, command.bytes.start + 3, command.bytes.end);
    r.u32().ok()?;
    let key = r.cstring().ok()?;
    let value = r.cstring().ok()?;
    Some(InfoPair { key, value })
}

// Type tags in a serialized Lua table.
const LUA_NUMBER: u8 = 0;
const LUA_STRING: u8 = 1;
const LUA_NIL: u8 = 2;
const LUA_BOOL: u8 = 3;
const LUA_TABLE: u8 = 4;
const LUA_TABLE_END: u8 = 5;

/// Nested tables deeper than this are treated as a corrupt file.
const MAX_LUA_DEPTH: usize = 32;

/// Walks an army's settings table and returns where its `PlayerName` is.
fn find_player_name(data: &[u8], blob: Range<usize>) -> Result<Option<Range<usize>>> {
    let mut r = Reader::new(data, blob.start, blob.end);
    if r.u8()? != LUA_TABLE {
        return Err(r.error_at(blob.start, "army settings are not a table"));
    }
    let mut player_name = None;
    while r.peek()? != LUA_TABLE_END {
        let key = r.lua_value(0)?;
        let value = r.lua_value(0)?;
        if let (Some(key), Some(value)) = (key, value)
            && &data[key] == b"PlayerName"
        {
            player_name = Some(value);
        }
    }
    r.u8()?;
    if r.pos != blob.end {
        return Err(r.error_at(r.pos, "army settings have extra bytes"));
    }
    Ok(player_name)
}

/// Reads bytes from `data[pos..end]`, failing instead of reading past `end`.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8], pos: usize, end: usize) -> Self {
        Reader { data, pos, end }
    }

    fn error_at(&self, offset: usize, message: &str) -> ParseError {
        ParseError {
            offset,
            message: message.to_string(),
        }
    }

    fn take(&mut self, n: usize) -> Result<Range<usize>> {
        if n > self.end - self.pos {
            return Err(self.error_at(self.pos, "file ends too early"));
        }
        let start = self.pos;
        self.pos += n;
        Ok(start..self.pos)
    }

    fn peek(&self) -> Result<u8> {
        if self.pos >= self.end {
            return Err(self.error_at(self.pos, "file ends too early"));
        }
        Ok(self.data[self.pos])
    }

    fn u8(&mut self) -> Result<u8> {
        let b = self.take(1)?;
        Ok(self.data[b.start])
    }

    fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([
            self.data[b.start],
            self.data[b.start + 1],
        ]))
    }

    fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        let mut bytes = [0; 4];
        bytes.copy_from_slice(&self.data[b]);
        Ok(u32::from_le_bytes(bytes))
    }

    /// A zero-terminated string. Returns the text without the zero byte.
    fn cstring(&mut self) -> Result<Range<usize>> {
        let start = self.pos;
        let len = self.data[start..self.end]
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| self.error_at(start, "string has no end"))?;
        self.pos += len + 1;
        Ok(start..start + len)
    }

    /// Skips one Lua value. Returns the text range if the value is a string.
    fn lua_value(&mut self, depth: usize) -> Result<Option<Range<usize>>> {
        let start = self.pos;
        match self.u8()? {
            LUA_NUMBER => {
                self.take(4)?;
            }
            LUA_STRING => return Ok(Some(self.cstring()?)),
            LUA_NIL => {}
            LUA_BOOL => {
                self.u8()?;
            }
            LUA_TABLE => {
                if depth >= MAX_LUA_DEPTH {
                    return Err(self.error_at(start, "tables are nested too deeply"));
                }
                while self.peek()? != LUA_TABLE_END {
                    self.lua_value(depth + 1)?;
                    self.lua_value(depth + 1)?;
                }
                self.u8()?;
            }
            other => return Err(self.error_at(start, &format!("unknown Lua type {other}"))),
        }
        Ok(None)
    }
}
