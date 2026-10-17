//! Messages between the window (runs as the user) and the helper (runs as root through pkexec),
//! one JSON object per line over the helper's stdin and stdout.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Write};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Rescan keyboards.
    List,
    /// Stream key presses from this keyboard without capturing it.
    Watch { keyboard: String },
    /// Capture the watched keyboard and send these (pressed, sent) key pairs to the game.
    /// While playing, a new Play just swaps the pairs.
    Play { pairs: Vec<(u16, u16)> },
    /// Give the keyboard back to the desktop and stop sending.
    Stop,
    /// Stop keyd until the helper exits, then start it again.
    PauseKeyd,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Checks { keyd_running: bool, virtual_keyboard_error: Option<String> },
    Keyboards { list: Vec<Keyboard> },
    /// The keyboard now streaming key events, or None after a failed Watch or an unplug.
    Watching { keyboard: Option<String> },
    /// Every press and release on the watched keyboard, for the tester and for rebinding.
    Key { code: u16, down: bool },
    Mode { mode: Mode },
    Error { message: String },
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
pub struct Keyboard {
    /// USB port plus serial; stable while the keyboard stays in the same port.
    pub id: String,
    pub name: String,
    /// Why it can't be used, e.g. another program holds it.
    pub blocked: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Stopped,
    /// Captured: keys go to the game as Controller 2 keys.
    Game,
    /// Playing was started, but F12 handed the keyboard back to the desktop.
    Desktop,
}

pub fn send(out: &mut impl Write, msg: &impl Serialize) -> io::Result<()> {
    serde_json::to_writer(&mut *out, msg).map_err(io::Error::other)?;
    out.write_all(b"\n")?;
    out.flush()
}

pub fn parse<T: DeserializeOwned>(line: &str) -> io::Result<T> {
    serde_json::from_str(line).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_as_lines() {
        let mut buf = Vec::new();
        send(&mut buf, &Command::Play { pairs: vec![(36, 110)] }).unwrap();
        send(&mut buf, &Event::Mode { mode: Mode::Game }).unwrap();
        let text = String::from_utf8(buf).unwrap();
        let lines: Vec<_> = text.lines().collect();
        assert_eq!(lines[0], r#"{"cmd":"play","pairs":[[36,110]]}"#);
        assert_eq!(parse::<Command>(lines[0]).unwrap(), Command::Play { pairs: vec![(36, 110)] });
        assert_eq!(parse::<Event>(lines[1]).unwrap(), Event::Mode { mode: Mode::Game });
        assert!(parse::<Command>(r#"{"cmd":"format_disk"}"#).is_err());
    }
}
