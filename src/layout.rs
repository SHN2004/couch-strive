//! Player 2's layout: which keys they press for each action and which key the game gets, kept in a config file.

use evdev::KeyCode as K;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf, str::FromStr};

/// Switches Player 2's keyboard between the game and the desktop, so it can never be bound.
pub const TOGGLE: K = K::KEY_F12;

pub struct Action {
    pub name: &'static str,
    pub group: &'static str,
    /// GGST's default Controller 2 key for this action.
    pub sends: K,
    defaults: &'static [K],
}

/// GGST confirms Controller 2 menus with Insert and goes back with Page Up, the same keys as
/// Punch and Dust, so those actions also carry the confirm and back keys.
pub const ACTIONS: [Action; 10] = [
    Action { name: "Up", group: "Move", sends: K::KEY_UP, defaults: &[K::KEY_W, K::KEY_UP] },
    Action { name: "Down", group: "Move", sends: K::KEY_DOWN, defaults: &[K::KEY_S, K::KEY_DOWN] },
    Action { name: "Left", group: "Move", sends: K::KEY_LEFT, defaults: &[K::KEY_A, K::KEY_LEFT] },
    Action { name: "Right", group: "Move", sends: K::KEY_RIGHT, defaults: &[K::KEY_D, K::KEY_RIGHT] },
    Action { name: "Punch / confirm", group: "Attack", sends: K::KEY_INSERT, defaults: &[K::KEY_J, K::KEY_U] },
    Action { name: "Kick", group: "Attack", sends: K::KEY_DELETE, defaults: &[K::KEY_I] },
    Action { name: "Slash", group: "Attack", sends: K::KEY_HOME, defaults: &[K::KEY_K] },
    Action { name: "Heavy Slash", group: "Attack", sends: K::KEY_END, defaults: &[K::KEY_L] },
    Action { name: "Dust / back", group: "Attack", sends: K::KEY_PAGEUP, defaults: &[K::KEY_O, K::KEY_BACKSPACE] },
    Action { name: "Dash", group: "Other", sends: K::KEY_PAGEDOWN, defaults: &[K::KEY_LEFTSHIFT, K::KEY_RIGHTSHIFT] },
];

/// Keys a normal keyboard types with, so Player 1's own keyboard may press them too.
const TYPING: &[K] = &[
    K::KEY_SPACE, K::KEY_ENTER, K::KEY_TAB, K::KEY_BACKSPACE,
    K::KEY_A, K::KEY_B, K::KEY_C, K::KEY_D, K::KEY_E, K::KEY_F, K::KEY_G, K::KEY_H, K::KEY_I, K::KEY_J,
    K::KEY_K, K::KEY_L, K::KEY_M, K::KEY_N, K::KEY_O, K::KEY_P, K::KEY_Q, K::KEY_R, K::KEY_S, K::KEY_T,
    K::KEY_U, K::KEY_V, K::KEY_W, K::KEY_X, K::KEY_Y, K::KEY_Z,
    K::KEY_1, K::KEY_2, K::KEY_3, K::KEY_4, K::KEY_5, K::KEY_6, K::KEY_7, K::KEY_8, K::KEY_9, K::KEY_0,
];
const MODIFIERS: &[K] = &[
    K::KEY_LEFTSHIFT, K::KEY_RIGHTSHIFT, K::KEY_LEFTCTRL, K::KEY_RIGHTCTRL, K::KEY_LEFTALT, K::KEY_RIGHTALT,
];

/// Keys the game can be sent, grouped for the picker. A virtual keyboard can't add keys after it
/// is created, so the helper declares all of these up front. F12 is left out: it is the toggle and
/// Steam's screenshot key.
pub const OUTPUT_GROUPS: [(&str, &[K]); 6] = [
    ("Arrows", &[K::KEY_UP, K::KEY_DOWN, K::KEY_LEFT, K::KEY_RIGHT]),
    ("Editing keys", &[K::KEY_INSERT, K::KEY_DELETE, K::KEY_HOME, K::KEY_END, K::KEY_PAGEUP, K::KEY_PAGEDOWN]),
    (
        "Numpad",
        &[
            K::KEY_KP0, K::KEY_KP1, K::KEY_KP2, K::KEY_KP3, K::KEY_KP4, K::KEY_KP5, K::KEY_KP6, K::KEY_KP7,
            K::KEY_KP8, K::KEY_KP9, K::KEY_KPDOT, K::KEY_KPENTER, K::KEY_KPPLUS, K::KEY_KPMINUS,
            K::KEY_KPASTERISK, K::KEY_KPSLASH,
        ],
    ),
    (
        "Function keys",
        &[
            K::KEY_F1, K::KEY_F2, K::KEY_F3, K::KEY_F4, K::KEY_F5, K::KEY_F6, K::KEY_F7, K::KEY_F8, K::KEY_F9,
            K::KEY_F10, K::KEY_F11,
        ],
    ),
    ("Typing keys", TYPING),
    ("Modifiers", MODIFIERS),
];

pub fn outputs() -> impl Iterator<Item = K> {
    OUTPUT_GROUPS.iter().flat_map(|(_, keys)| keys.iter().copied())
}

/// Whether Player 1's own keyboard could also send this game key.
pub fn shared_with_player_1(key: K) -> bool {
    TYPING.contains(&key) || MODIFIERS.contains(&key)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    /// Keys Player 2 presses.
    pub keys: Vec<K>,
    /// The key the game gets.
    pub sends: K,
}

/// One binding per action, in `ACTIONS` order.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout(pub Vec<Binding>);

impl Default for Layout {
    fn default() -> Self {
        Layout(ACTIONS.iter().map(|a| Binding { keys: a.defaults.to_vec(), sends: a.sends }).collect())
    }
}

impl Layout {
    /// (key Player 2 presses, key sent to the game) pairs, as the helper takes them.
    pub fn pairs(&self) -> Vec<(u16, u16)> {
        self.0.iter().flat_map(|b| b.keys.iter().map(|k| (k.0, b.sends.0))).collect()
    }

    pub fn action_of(&self, key: K) -> Option<usize> {
        self.0.iter().position(|b| b.keys.contains(&key))
    }

    /// Binds `key` to `action`, taking it off any other action so each key does one thing.
    /// Returns false for the toggle key.
    pub fn bind(&mut self, action: usize, key: K) -> bool {
        if key == TOGGLE {
            return false;
        }
        for binding in &mut self.0 {
            binding.keys.retain(|&k| k != key);
        }
        self.0[action].keys.push(key);
        true
    }

    pub fn unbind(&mut self, action: usize, key: K) {
        self.0[action].keys.retain(|&k| k != key);
    }

    /// Changes the game key; refuses keys the virtual keyboard can't send.
    pub fn set_sends(&mut self, action: usize, key: K) -> bool {
        let ok = outputs().any(|k| k == key);
        if ok {
            self.0[action].sends = key;
        }
        ok
    }

    /// Another action sending the same game key, which the game can't tell apart.
    pub fn sharing_sends(&self, action: usize) -> Option<usize> {
        let sends = self.0[action].sends;
        (0..self.0.len()).find(|&i| i != action && self.0[i].sends == sends)
    }
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    chars.next().map_or_else(String::new, |first| first.to_string() + &chars.as_str().to_lowercase())
}

/// Short label for a key, e.g. "J", "Left Shift", "Page Up", "Num 7".
pub fn key_name(key: K) -> String {
    let raw = format!("{key:?}");
    let name = raw.strip_prefix("KEY_").unwrap_or(&raw);
    if let Some(pad) = name.strip_prefix("KP") {
        let symbol = match pad {
            "DOT" => ".".to_owned(),
            "ENTER" => "Enter".to_owned(),
            "PLUS" => "+".to_owned(),
            "MINUS" => "-".to_owned(),
            "ASTERISK" => "*".to_owned(),
            "SLASH" => "/".to_owned(),
            other => title_case(other),
        };
        return format!("Num {symbol}");
    }
    let special = match name {
        "UP" => "Up arrow",
        "DOWN" => "Down arrow",
        "LEFT" => "Left arrow",
        "RIGHT" => "Right arrow",
        "PAGEUP" => "Page Up",
        "PAGEDOWN" => "Page Down",
        "LEFTSHIFT" => "Left Shift",
        "RIGHTSHIFT" => "Right Shift",
        "LEFTCTRL" => "Left Ctrl",
        "RIGHTCTRL" => "Right Ctrl",
        "LEFTALT" => "Left Alt",
        "RIGHTALT" => "Right Alt",
        "CAPSLOCK" => "Caps Lock",
        "SEMICOLON" => ";",
        "APOSTROPHE" => "'",
        "COMMA" => ",",
        "DOT" => ".",
        "SLASH" => "/",
        "BACKSLASH" => "\\",
        "MINUS" => "-",
        "EQUAL" => "=",
        "LEFTBRACE" => "[",
        "RIGHTBRACE" => "]",
        "GRAVE" => "`",
        _ => return title_case(name),
    };
    special.to_owned()
}

/// What the window remembers between runs.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Config {
    /// Player 2's keyboard: the id ties it to a USB port, the name finds it on another port.
    pub keyboard_id: Option<String>,
    pub keyboard_name: Option<String>,
    /// Action name -> evdev key names Player 2 presses, e.g. "Kick": ["KEY_I"].
    pub layout: BTreeMap<String, Vec<String>>,
    /// Action name -> the evdev key name the game gets, e.g. "Kick": "KEY_DELETE".
    pub sends: BTreeMap<String, String>,
}

impl Config {
    fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
        base.join("couch-strive").join("config.json")
    }

    /// A missing file is a first run, not an error.
    pub fn load() -> io::Result<Config> {
        match fs::read(Self::path()) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = Self::path();
        fs::create_dir_all(path.parent().expect("config path has a parent"))?;
        fs::write(path, serde_json::to_vec_pretty(self).map_err(io::Error::other)?)
    }

    /// The saved layout. Missing actions keep their defaults; unknown key names, and game keys
    /// the virtual keyboard can't send, are dropped.
    pub fn layout(&self) -> Layout {
        let mut layout = Layout::default();
        for (i, action) in ACTIONS.iter().enumerate() {
            if let Some(names) = self.layout.get(action.name) {
                layout.0[i].keys = names.iter().filter_map(|n| K::from_str(n).ok()).filter(|&k| k != TOGGLE).collect();
            }
            if let Some(key) = self.sends.get(action.name).and_then(|n| K::from_str(n).ok()) {
                layout.set_sends(i, key);
            }
        }
        layout
    }

    pub fn set_layout(&mut self, layout: &Layout) {
        let names = |keys: &[K]| keys.iter().map(|k| format!("{k:?}")).collect();
        self.layout = ACTIONS.iter().zip(&layout.0).map(|(a, b)| (a.name.to_owned(), names(&b.keys))).collect();
        self.sends = ACTIONS.iter().zip(&layout.0).map(|(a, b)| (a.name.to_owned(), format!("{:?}", b.sends))).collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_controller_2() {
        let layout = Layout::default();
        let sends = |key: K| layout.action_of(key).map(|a| layout.0[a].sends);
        assert_eq!(sends(K::KEY_A), Some(K::KEY_LEFT));
        assert_eq!(sends(K::KEY_J), Some(K::KEY_INSERT));
        assert_eq!(sends(K::KEY_U), Some(K::KEY_INSERT));
        assert_eq!(sends(K::KEY_BACKSPACE), Some(K::KEY_PAGEUP));
        assert_eq!(sends(K::KEY_SPACE), None);
        assert!(layout.pairs().contains(&(K::KEY_I.0, K::KEY_DELETE.0)));
        // every default game key must be sendable, or the kernel would drop it
        assert!(ACTIONS.iter().all(|a| outputs().any(|k| k == a.sends)));
        assert!(!outputs().any(|k| k == TOGGLE));
    }

    #[test]
    fn binding_moves_a_key_and_refuses_the_toggle() {
        let mut layout = Layout::default();
        assert!(layout.bind(5, K::KEY_J)); // J: Punch -> Kick
        assert_eq!(layout.action_of(K::KEY_J), Some(5));
        assert_eq!(layout.0[4].keys, vec![K::KEY_U]);
        assert!(!layout.bind(0, TOGGLE));
        assert_eq!(layout.action_of(TOGGLE), None);
    }

    #[test]
    fn game_keys_change_and_clashes_show() {
        let mut layout = Layout::default();
        assert!(layout.set_sends(5, K::KEY_KP5));
        assert!(layout.pairs().contains(&(K::KEY_I.0, K::KEY_KP5.0)));
        assert!(!layout.set_sends(5, TOGGLE)); // not a sendable game key
        assert!(layout.set_sends(0, K::KEY_SPACE));
        assert!(shared_with_player_1(K::KEY_SPACE) && !shared_with_player_1(K::KEY_INSERT));
        assert_eq!(layout.sharing_sends(5), None);
        assert!(layout.set_sends(6, K::KEY_KP5));
        assert_eq!(layout.sharing_sends(5), Some(6));
    }

    #[test]
    fn config_round_trips_the_layout() {
        let mut layout = Layout::default();
        layout.bind(9, K::KEY_SPACE);
        layout.unbind(0, K::KEY_UP);
        layout.set_sends(5, K::KEY_KP5);
        let mut config = Config::default();
        config.set_layout(&layout);
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"Dash\":[\"KEY_LEFTSHIFT\",\"KEY_RIGHTSHIFT\",\"KEY_SPACE\"]"));
        assert!(json.contains("\"Kick\":\"KEY_KP5\""));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layout(), layout);
        // files from before game keys were editable, and hand edits with typos, still load
        let old: Config = serde_json::from_str(r#"{"keyboard_id":null,"keyboard_name":null,"layout":{}}"#).unwrap();
        assert_eq!(old.layout(), Layout::default());
        let typo: Config =
            serde_json::from_str(r#"{"layout":{"Kick":["KEY_NOPE","KEY_Z"]},"sends":{"Kick":"KEY_MUTE"}}"#).unwrap();
        assert_eq!(typo.layout().0[5], Binding { keys: vec![K::KEY_Z], sends: K::KEY_DELETE });
    }

    #[test]
    fn key_names_read_well() {
        assert_eq!(key_name(K::KEY_J), "J");
        assert_eq!(key_name(K::KEY_LEFTSHIFT), "Left Shift");
        assert_eq!(key_name(K::KEY_BACKSPACE), "Backspace");
        assert_eq!(key_name(K::KEY_PAGEUP), "Page Up");
        assert_eq!(key_name(K::KEY_F12), "F12");
        assert_eq!(key_name(K::KEY_KP7), "Num 7");
        assert_eq!(key_name(K::KEY_KPENTER), "Num Enter");
    }
}
