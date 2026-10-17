//! Root side, started by the window through pkexec. It reads Commands on stdin and writes Events on
//! stdout. When stdin closes (the window quit or crashed) it releases the keyboard, starts keyd
//! again if it paused it, and exits.

use crate::layout::{ACTIONS, TOGGLE};
use crate::protocol::{self, Command, Event, Keyboard, Mode};
use evdev::{uinput::VirtualDevice, AttributeSet, Device, EventSummary, EventType, InputEvent, KeyCode as K};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader, StdoutLock},
    os::fd::{AsFd, AsRawFd},
    process::{self, Stdio},
    thread,
    time::Duration,
};

/// Counts the keys holding each output, so two keys on one output (J and U on Insert) don't cut
/// each other off. Returns whether the output went down or up.
fn press(counts: &mut HashMap<K, u8>, out: K, down: bool) -> bool {
    let n = counts.entry(out).or_default();
    let was = *n > 0;
    *n = if down { n.saturating_add(1) } else { n.saturating_sub(1) };
    was != (*n > 0)
}

/// The virtual keyboard that types GGST's Controller 2 keys.
struct Keys {
    dev: VirtualDevice,
    down: HashMap<K, u8>,
}

impl Keys {
    fn new() -> io::Result<Self> {
        let keys: AttributeSet<K> = ACTIONS.iter().map(|a| a.sends).collect();
        let dev = VirtualDevice::builder()?.name("couch-strive Player 2 keys").with_keys(&keys)?.build()?;
        Ok(Keys { dev, down: HashMap::new() })
    }

    fn set(&mut self, out: K, down: bool) -> io::Result<()> {
        if !press(&mut self.down, out, down) {
            return Ok(());
        }
        self.dev.emit(&[InputEvent::new(EventType::KEY.0, out.0, down as i32)])
    }

    /// Lets go of everything, so no key stays held once the keyboard is back on the desktop.
    fn release(&mut self) -> io::Result<()> {
        let held: Vec<_> = self
            .down
            .drain()
            .filter(|&(_, n)| n > 0)
            .map(|(k, _)| InputEvent::new(EventType::KEY.0, k.0, 0))
            .collect();
        if held.is_empty() { Ok(()) } else { self.dev.emit(&held) }
    }
}

/// Physical keyboards: every event node that can type letters, grouped by USB port and serial,
/// since some keyboards (NKRO ones) send letters from more than one node.
fn keyboards() -> Vec<(Keyboard, Vec<Device>)> {
    let mut found: Vec<(Keyboard, Vec<Device>)> = Vec::new();
    for (_, mut dev) in evdev::enumerate() {
        if !dev.supported_keys().is_some_and(|k| k.contains(K::KEY_A)) {
            continue;
        }
        // Virtual keyboards (no port, e.g. keyd's or our own) merge or fake input, so they never belong to a player.
        // shortcut: two keyboards on one wireless receiver look like one keyboard; split on the ":N" suffix if that comes up.
        let Some(port) = dev.physical_path().and_then(|p| p.split('/').next()).filter(|p| !p.is_empty()) else {
            continue;
        };
        let id = format!("{port}|{}", dev.unique_name().unwrap_or_default());
        // A remapper holding the grab (keyd, input-remapper) would hide every key from us.
        let blocked = dev.grab().and_then(|()| dev.ungrab()).err().map(|e| match e.raw_os_error() {
            Some(libc::EBUSY) => "used by another program, e.g. keyd".to_owned(),
            _ => e.to_string(),
        });
        let name = dev.name().unwrap_or("Keyboard").to_owned();
        match found.iter_mut().find(|(kb, _)| kb.id == id) {
            Some((kb, devs)) => {
                kb.blocked = kb.blocked.take().or(blocked);
                devs.push(dev);
            }
            None => found.push((Keyboard { id, name, blocked }, vec![dev])),
        }
    }
    found
}

fn systemctl(args: &[&str]) -> bool {
    // stdout is our protocol channel, so systemctl must not write to it
    process::Command::new("systemctl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn wait(fds: &mut [libc::pollfd]) -> io::Result<()> {
    // SAFETY: fds is a live pollfd array of the length passed.
    if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) } < 0 {
        let e = io::Error::last_os_error();
        if e.kind() != io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
    Ok(())
}

struct Helper {
    out: StdoutLock<'static>,
    keys: Option<Keys>,
    keys_error: Option<String>,
    /// Player 2's keyboard id and its event nodes.
    watched: Option<(String, Vec<Device>)>,
    mode: Mode,
    pairs: HashMap<u16, u16>,
    keyd_paused: bool,
}

pub fn run() -> io::Result<()> {
    // Read our own copy of stdin through a single buffer, so poll() and the buffer always agree.
    let mut commands = BufReader::new(File::from(io::stdin().as_fd().try_clone_to_owned()?));
    let (keys, keys_error) = match Keys::new() {
        Ok(keys) => (Some(keys), None),
        Err(e) => (None, Some(format!("/dev/uinput: {e}"))),
    };
    let mut helper = Helper {
        out: io::stdout().lock(),
        keys,
        keys_error,
        watched: None,
        mode: Mode::Stopped,
        pairs: HashMap::new(),
        keyd_paused: false,
    };
    let result = helper.serve(&mut commands);
    if helper.keyd_paused {
        systemctl(&["start", "keyd"]);
    }
    result
}

impl Helper {
    fn serve(&mut self, commands: &mut BufReader<File>) -> io::Result<()> {
        self.checks()?;
        self.list()?;
        loop {
            let mut fds: Vec<_> = std::iter::once(commands.get_ref().as_raw_fd())
                .chain(self.watched.iter().flat_map(|(_, devs)| devs.iter().map(|d| d.as_raw_fd())))
                .map(|fd| libc::pollfd { fd, events: libc::POLLIN, revents: 0 })
                .collect();
            if commands.buffer().is_empty() {
                wait(&mut fds)?;
            }
            if !commands.buffer().is_empty() || fds[0].revents != 0 {
                let mut line = String::new();
                if commands.read_line(&mut line)? == 0 {
                    return Ok(()); // the window closed
                }
                match protocol::parse(&line) {
                    Ok(cmd) => self.handle(cmd)?,
                    Err(e) => self.error(format!("Bad command: {e}"))?,
                }
                continue;
            }
            self.forward(&fds[1..])?;
        }
    }

    fn send(&mut self, event: &Event) -> io::Result<()> {
        protocol::send(&mut self.out, event)
    }

    fn error(&mut self, message: impl Into<String>) -> io::Result<()> {
        self.send(&Event::Error { message: message.into() })
    }

    fn checks(&mut self) -> io::Result<()> {
        let event = Event::Checks {
            keyd_running: systemctl(&["is-active", "--quiet", "keyd"]),
            virtual_keyboard_error: self.keys_error.clone(),
        };
        self.send(&event)
    }

    fn list(&mut self) -> io::Result<()> {
        let list = keyboards().into_iter().map(|(kb, _)| kb).collect();
        self.send(&Event::Keyboards { list })
    }

    fn set_mode(&mut self, mode: Mode) -> io::Result<()> {
        self.mode = mode;
        self.send(&Event::Mode { mode })
    }

    /// Grabs (game) or releases (desktop) Player 2's keyboard.
    fn grab(&mut self, on: bool) -> io::Result<()> {
        if let Some((_, devs)) = &mut self.watched {
            for dev in devs.iter_mut() {
                if on {
                    // Grabbing while a key is down would leave it stuck on the desktop.
                    while dev.get_key_state()?.iter().next().is_some() {
                        thread::sleep(Duration::from_millis(10));
                    }
                    dev.grab()?;
                } else {
                    dev.ungrab()?;
                }
            }
        }
        match (&mut self.keys, on) {
            (Some(keys), false) => keys.release(),
            _ => Ok(()),
        }
    }

    fn stop(&mut self) -> io::Result<()> {
        if self.mode == Mode::Game {
            self.grab(false)?;
        }
        if self.mode != Mode::Stopped {
            self.set_mode(Mode::Stopped)?;
        }
        Ok(())
    }

    fn handle(&mut self, cmd: Command) -> io::Result<()> {
        match cmd {
            // our own grab would make Player 2's keyboard look blocked
            Command::List if self.mode == Mode::Game => self.error("Stop before refreshing the keyboard list"),
            Command::List => self.list(),
            Command::Watch { keyboard } => {
                self.stop()?;
                self.watched = None;
                let found = keyboards().into_iter().find(|(kb, _)| kb.id == keyboard);
                match found {
                    Some((kb, devs)) if kb.blocked.is_none() => {
                        for dev in &devs {
                            dev.set_nonblocking(true)?;
                        }
                        self.watched = Some((keyboard.clone(), devs));
                        self.send(&Event::Watching { keyboard: Some(keyboard) })
                    }
                    Some((kb, _)) => {
                        self.send(&Event::Watching { keyboard: None })?;
                        self.error(format!("{} is {}", kb.name, kb.blocked.unwrap_or_default()))
                    }
                    None => {
                        self.send(&Event::Watching { keyboard: None })?;
                        self.error("That keyboard is no longer connected")
                    }
                }
            }
            Command::Play { pairs } => {
                self.pairs = pairs.into_iter().collect();
                if self.watched.is_none() {
                    return self.error("Pick Player 2's keyboard first");
                }
                if self.keys.is_none() {
                    return self.error("The virtual keyboard is unavailable");
                }
                if self.mode == Mode::Stopped {
                    self.grab(true)?;
                    self.set_mode(Mode::Game)?;
                }
                Ok(())
            }
            Command::Stop => self.stop(),
            Command::PauseKeyd => {
                if systemctl(&["stop", "keyd"]) {
                    self.keyd_paused = true;
                } else {
                    self.error("Could not stop keyd")?;
                }
                self.checks()?;
                self.list()
            }
        }
    }

    /// Reads Player 2's keyboard: reports every key, handles F12, and sends mapped keys while in game.
    fn forward(&mut self, ready: &[libc::pollfd]) -> io::Result<()> {
        let Some((_, devs)) = &mut self.watched else { return Ok(()) };
        let mut events = Vec::new();
        let mut lost = None;
        for (dev, fd) in devs.iter_mut().zip(ready) {
            if fd.revents == 0 {
                continue;
            }
            match dev.fetch_events() {
                Ok(evs) => events.extend(evs.filter_map(|ev| match ev.destructure() {
                    EventSummary::Key(_, key, value) => Some((key, value)),
                    _ => None,
                })),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) => lost = Some(e),
            }
        }
        if let Some(e) = lost {
            self.watched = None;
            if let Some(keys) = &mut self.keys {
                keys.release()?;
            }
            if self.mode != Mode::Stopped {
                self.set_mode(Mode::Stopped)?;
            }
            self.send(&Event::Watching { keyboard: None })?;
            return self.error(format!("Lost Player 2's keyboard: {e}"));
        }
        for (key, value) in events {
            if value == 2 {
                continue; // autorepeat
            }
            self.send(&Event::Key { code: key.0, down: value == 1 })?;
            // F12 acts on release, so the desktop always sees whole key presses
            match (key, self.mode) {
                (TOGGLE, Mode::Game) if value == 0 => {
                    self.grab(false)?;
                    self.set_mode(Mode::Desktop)?;
                }
                (TOGGLE, Mode::Desktop) if value == 0 => {
                    self.grab(true)?;
                    self.set_mode(Mode::Game)?;
                }
                (_, Mode::Game) => {
                    if let (Some(keys), Some(&out)) = (&mut self.keys, self.pairs.get(&key.0)) {
                        keys.set(K::new(out), value == 1)?;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_key_stays_held() {
        let (mut counts, insert) = (HashMap::new(), K::KEY_INSERT);
        assert!(press(&mut counts, insert, true)); // U down: Insert down
        assert!(!press(&mut counts, insert, true)); // J down: already down
        assert!(!press(&mut counts, insert, false)); // U up: J still holds Insert
        assert!(press(&mut counts, insert, false)); // J up: Insert up
        assert!(!press(&mut counts, insert, false)); // stray release changes nothing
    }
}
