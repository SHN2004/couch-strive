//! Second keyboard -> Guilty Gear Strive's built-in Player 2 keyboard keys, for local versus.
//! With "Keyboard Control Type" set to two players, GGST gives Controller 2 its own keys (arrows,
//! Insert, Delete, ...). kbsplit captures only Player 2's keyboard and replays it as those keys on a
//! virtual keyboard, so both players use the same layout on their own keyboard. Player 1's keyboard
//! is left alone. Linux take on KeyboardSplitterXbox: an evdev grab replaces the Interception driver.

use evdev::{uinput::VirtualDevice, AttributeSet, Device, EventSummary, EventType, InputEvent, KeyCode as K};
use std::{collections::HashMap, io, os::fd::AsRawFd, thread, time::Duration};

const TOGGLE: K = K::KEY_F12;
const OUTPUTS: [K; 10] = [
    K::KEY_UP, K::KEY_DOWN, K::KEY_LEFT, K::KEY_RIGHT, K::KEY_INSERT,
    K::KEY_DELETE, K::KEY_HOME, K::KEY_END, K::KEY_PAGEUP, K::KEY_PAGEDOWN,
];
const LAYOUT: &str = "Player 2: WASD or arrows = move, J I K L O = P K S HS D, Shift = Dash (sends Page Down),
          U = confirm (same key as Punch), Backspace = back (same key as Dust).
F12 on Player 2's keyboard switches it between the game and the desktop; Ctrl+C here quits.";

/// Player 2's keys -> GGST's default Controller 2 keys, mirroring Player 1's J I K L O layout.
fn map(key: K) -> Option<K> {
    Some(match key {
        K::KEY_W | K::KEY_UP => K::KEY_UP,
        K::KEY_S | K::KEY_DOWN => K::KEY_DOWN,
        K::KEY_A | K::KEY_LEFT => K::KEY_LEFT,
        K::KEY_D | K::KEY_RIGHT => K::KEY_RIGHT,
        // Controller 2's menu Select and Return default to Insert and Page Up, the same keys as
        // Punch and Dust, so U (confirm) and Backspace (back) share them like on Player 1's keyboard.
        K::KEY_J | K::KEY_U => K::KEY_INSERT, // Punch
        K::KEY_I => K::KEY_DELETE, // Kick
        K::KEY_K => K::KEY_HOME, // Slash
        K::KEY_L => K::KEY_END, // Heavy Slash
        K::KEY_O | K::KEY_BACKSPACE => K::KEY_PAGEUP, // Dust
        K::KEY_LEFTSHIFT | K::KEY_RIGHTSHIFT => K::KEY_PAGEDOWN, // Dash has no Controller 2 default shown
        _ => return None,
    })
}

/// Counts the keys holding each output, so two keys on one output (J and U on Insert) don't cut
/// each other off. Returns whether the output went down or up.
fn press(counts: &mut HashMap<K, u8>, out: K, down: bool) -> bool {
    let n = counts.entry(out).or_default();
    let was = *n > 0;
    *n = if down { n.saturating_add(1) } else { n.saturating_sub(1) };
    was != (*n > 0)
}

struct Keys {
    dev: VirtualDevice,
    down: HashMap<K, u8>,
}

impl Keys {
    fn new() -> io::Result<Self> {
        let keys: AttributeSet<K> = OUTPUTS.into_iter().collect();
        let dev = VirtualDevice::builder()?.name("kbsplit Player 2 keys").with_keys(&keys)?.build()?;
        Ok(Keys { dev, down: HashMap::new() })
    }

    fn set(&mut self, out: K, down: bool) -> io::Result<()> {
        if !press(&mut self.down, out, down) {
            return Ok(());
        }
        self.dev.emit(&[InputEvent::new(EventType::KEY.0, out.0, down as i32)])
    }

    /// Lets go of everything, so no key stays held while the keyboard is on the desktop.
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

/// Every event node that can type letters, keyed by its physical keyboard. Some keyboards
/// (NKRO ones) send letters from more than one node, so all of a keyboard's nodes get grabbed.
fn keyboards() -> io::Result<Vec<(String, Device)>> {
    let mut found = Vec::new();
    for (_, mut dev) in evdev::enumerate() {
        if !dev.supported_keys().is_some_and(|k| k.contains(K::KEY_A)) {
            continue;
        }
        // USB port without the "/inputN" interface, plus uniq to split Bluetooth keyboards on one adapter.
        // Virtual keyboards (no port, e.g. keyd's) merge or fake input, so they never belong to a player.
        // shortcut: two keyboards on one wireless receiver look like one keyboard; split on the ":N" suffix if that comes up.
        let Some(port) = dev.physical_path().and_then(|p| p.split('/').next()).filter(|p| !p.is_empty()) else {
            continue;
        };
        let id = format!("{port}|{}", dev.unique_name().unwrap_or_default());
        // A remapper holding the grab (keyd, input-remapper) hides every key from us, so fail loudly.
        if let Err(e) = dev.grab().and_then(|()| dev.ungrab()) {
            let name = dev.name().unwrap_or("a keyboard");
            let msg = format!("{name} is captured by another program ({e}); stop it first, e.g. sudo systemctl stop keyd");
            return Err(io::Error::new(e.kind(), msg));
        }
        dev.set_nonblocking(true)?;
        found.push((id, dev));
    }
    if found.is_empty() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, "no readable keyboards; run with sudo"));
    }
    Ok(found)
}

/// Blocks until any keyboard has input, then returns (index into `boards`, key, value) for each key event.
fn next_keys<T>(boards: &mut [(T, Device)]) -> io::Result<Vec<(usize, K, i32)>> {
    let mut fds: Vec<_> = boards
        .iter()
        .map(|(_, dev)| libc::pollfd { fd: dev.as_raw_fd(), events: libc::POLLIN, revents: 0 })
        .collect();
    // SAFETY: fds is a live pollfd array of the length passed.
    if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) } < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::Interrupted { Ok(Vec::new()) } else { Err(e) };
    }
    let mut keys = Vec::new();
    for (i, ((_, dev), fd)) in boards.iter_mut().zip(&fds).enumerate() {
        if fd.revents == 0 {
            continue;
        }
        match dev.fetch_events() {
            Ok(events) => keys.extend(events.filter_map(|ev| match ev.destructure() {
                EventSummary::Key(_, key, value) => Some((i, key, value)),
                _ => None,
            })),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
    }
    Ok(keys)
}

/// Returns the keyboard on which SPACE is released next.
/// Waiting for the release means the desktop never misses a key-up when we grab.
fn pick(boards: &mut [(String, Device)]) -> io::Result<String> {
    println!("Player 2: press SPACE on your keyboard");
    loop {
        for (i, key, value) in next_keys(boards)? {
            if (key, value) == (K::KEY_SPACE, 0) {
                return Ok(boards[i].0.clone());
            }
        }
    }
}

/// Grabs (game) or releases (desktop) Player 2's keyboard.
fn capture(boards: &mut [(String, Device)], keys: &mut Keys, on: bool) -> io::Result<()> {
    for (_, dev) in boards.iter_mut() {
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
    if on {
        println!("GAME: Player 2's keyboard plays as GGST Controller 2. F12 = back to desktop.");
    } else {
        keys.release()?;
        println!("DESKTOP: Player 2's keyboard types normally. F12 = back to game, Ctrl+C = quit.");
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let mut boards = keyboards()?;
    let p2 = pick(&mut boards)?;
    // Only Player 2's keyboard nodes are kept; Player 1 types into the game directly.
    let mut boards: Vec<_> = boards.into_iter().filter(|(id, _)| *id == p2).collect();
    let mut keys = Keys::new()?;
    println!("{LAYOUT}");
    let mut captured = true;
    capture(&mut boards, &mut keys, captured)?;
    loop {
        let mut toggle = false;
        for (_, key, value) in next_keys(&mut boards)? {
            match (key, map(key), value) {
                (TOGGLE, _, 0) => toggle = true, // on release, so the desktop always sees whole key presses
                (_, Some(out), 0 | 1) if captured => keys.set(out, value == 1)?, // 2 is autorepeat
                _ => {}
            }
        }
        if toggle {
            captured = !captured;
            capture(&mut boards, &mut keys, captured)?;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_matches_controller_2() {
        assert_eq!(map(K::KEY_A), Some(K::KEY_LEFT));
        assert_eq!(map(K::KEY_A), map(K::KEY_LEFT));
        assert_eq!(map(K::KEY_J), Some(K::KEY_INSERT));
        assert_eq!(map(K::KEY_U), map(K::KEY_J));
        assert_eq!(map(K::KEY_BACKSPACE), Some(K::KEY_PAGEUP));
        assert_eq!(map(K::KEY_SPACE), None);
        assert_eq!(map(TOGGLE), None);
        // every output must be declared on the virtual keyboard, or the kernel drops it
        assert!((0..0x100).filter_map(|c| map(K::new(c))).all(|out| OUTPUTS.contains(&out)));
    }

    #[test]
    fn shared_key_stays_held() {
        let (mut counts, insert) = (HashMap::new(), K::KEY_INSERT);
        assert!(press(&mut counts, insert, true)); // U down: Insert down
        assert!(!press(&mut counts, insert, true)); // J down: already down
        assert!(!press(&mut counts, insert, false)); // U up: J still holds Insert
        assert!(press(&mut counts, insert, false)); // J up: Insert up
        assert!(!press(&mut counts, insert, false)); // stray release changes nothing
    }

    /// Needs write access to /dev/uinput: creates the virtual keyboard for 4 s without typing anything.
    #[test]
    #[ignore]
    fn virtual_keyboard_smoke() {
        let _keys = Keys::new().unwrap();
        thread::sleep(Duration::from_secs(4));
    }
}
