//! Player 2's layout: which keys they press for each GGST Controller 2 action, kept in a config file.

use evdev::KeyCode as K;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, io, path::PathBuf, str::FromStr};

/// Switches Player 2's keyboard between the game and the desktop, so it can never be bound.
pub const TOGGLE: K = K::KEY_F12;

pub struct Action {
    pub name: &'static str,
    /// GGST's default Controller 2 key for this action.
    pub sends: K,
    defaults: &'static [K],
}

/// GGST confirms Controller 2 menus with Insert and goes back with Page Up, the same keys as
/// Punch and Dust, so those actions also carry the confirm and back keys.
pub const ACTIONS: [Action; 10] = [
    Action { name: "Up", sends: K::KEY_UP, defaults: &[K::KEY_W, K::KEY_UP] },
    Action { name: "Down", sends: K::KEY_DOWN, defaults: &[K::KEY_S, K::KEY_DOWN] },
    Action { name: "Left", sends: K::KEY_LEFT, defaults: &[K::KEY_A, K::KEY_LEFT] },
    Action { name: "Right", sends: K::KEY_RIGHT, defaults: &[K::KEY_D, K::KEY_RIGHT] },
    Action { name: "Punch / confirm", sends: K::KEY_INSERT, defaults: &[K::KEY_J, K::KEY_U] },
    Action { name: "Kick", sends: K::KEY_DELETE, defaults: &[K::KEY_I] },
    Action { name: "Slash", sends: K::KEY_HOME, defaults: &[K::KEY_K] },
    Action { name: "Heavy Slash", sends: K::KEY_END, defaults: &[K::KEY_L] },
    Action { name: "Dust / back", sends: K::KEY_PAGEUP, defaults: &[K::KEY_O, K::KEY_BACKSPACE] },
    Action { name: "Dash", sends: K::KEY_PAGEDOWN, defaults: &[K::KEY_LEFTSHIFT, K::KEY_RIGHTSHIFT] },
];

/// Keys Player 2 presses for each action, in `ACTIONS` order.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout(pub Vec<Vec<K>>);

impl Default for Layout {
    fn default() -> Self {
        Layout(ACTIONS.iter().map(|a| a.defaults.to_vec()).collect())
    }
}

impl Layout {
    /// (key Player 2 presses, key sent to the game) pairs, as the helper takes them.
    pub fn pairs(&self) -> Vec<(u16, u16)> {
        self.0.iter().zip(&ACTIONS).flat_map(|(keys, a)| keys.iter().map(|k| (k.0, a.sends.0))).collect()
    }

    pub fn action_of(&self, key: K) -> Option<usize> {
        self.0.iter().position(|keys| keys.contains(&key))
    }

    /// Binds `key` to `action`, taking it off any other action so each key does one thing.
    /// Returns false for the toggle key.
    pub fn bind(&mut self, action: usize, key: K) -> bool {
        if key == TOGGLE {
            return false;
        }
        for keys in &mut self.0 {
            keys.retain(|&k| k != key);
        }
        self.0[action].push(key);
        true
    }

    pub fn unbind(&mut self, action: usize, key: K) {
        self.0[action].retain(|&k| k != key);
    }
}

/// Short label for a key, e.g. "J", "Left Shift", "Page Up".
pub fn key_name(key: K) -> String {
    let raw = format!("{key:?}");
    let name = raw.strip_prefix("KEY_").unwrap_or(&raw);
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
        _ => "",
    };
    if !special.is_empty() {
        return special.to_owned();
    }
    let mut chars = name.chars();
    chars.next().map_or_else(String::new, |first| first.to_string() + &chars.as_str().to_lowercase())
}

/// What the window remembers between runs.
#[derive(Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Config {
    /// Player 2's keyboard: the id ties it to a USB port, the name finds it on another port.
    pub keyboard_id: Option<String>,
    pub keyboard_name: Option<String>,
    /// Action name -> evdev key names, e.g. "Kick": ["KEY_I"].
    pub layout: BTreeMap<String, Vec<String>>,
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

    /// Saved keys per action; actions missing from the file keep their defaults, unknown key names are dropped.
    pub fn layout(&self) -> Layout {
        let mut layout = Layout::default();
        for (keys, action) in layout.0.iter_mut().zip(&ACTIONS) {
            if let Some(names) = self.layout.get(action.name) {
                *keys = names.iter().filter_map(|n| K::from_str(n).ok()).filter(|&k| k != TOGGLE).collect();
            }
        }
        layout
    }

    pub fn set_layout(&mut self, layout: &Layout) {
        self.layout = ACTIONS
            .iter()
            .zip(&layout.0)
            .map(|(a, keys)| (a.name.to_owned(), keys.iter().map(|k| format!("{k:?}")).collect()))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_controller_2() {
        let layout = Layout::default();
        let sends = |key: K| layout.action_of(key).map(|a| ACTIONS[a].sends);
        assert_eq!(sends(K::KEY_A), Some(K::KEY_LEFT));
        assert_eq!(sends(K::KEY_J), Some(K::KEY_INSERT));
        assert_eq!(sends(K::KEY_U), Some(K::KEY_INSERT));
        assert_eq!(sends(K::KEY_BACKSPACE), Some(K::KEY_PAGEUP));
        assert_eq!(sends(K::KEY_SPACE), None);
        assert!(layout.pairs().contains(&(K::KEY_I.0, K::KEY_DELETE.0)));
    }

    #[test]
    fn binding_moves_a_key_and_refuses_the_toggle() {
        let mut layout = Layout::default();
        assert!(layout.bind(5, K::KEY_J)); // J: Punch -> Kick
        assert_eq!(layout.action_of(K::KEY_J), Some(5));
        assert_eq!(layout.0[4], vec![K::KEY_U]);
        assert!(!layout.bind(0, TOGGLE));
        assert_eq!(layout.action_of(TOGGLE), None);
    }

    #[test]
    fn config_round_trips_the_layout() {
        let mut layout = Layout::default();
        layout.bind(9, K::KEY_SPACE);
        layout.unbind(0, K::KEY_UP);
        let mut config = Config::default();
        config.set_layout(&layout);
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"Dash\":[\"KEY_LEFTSHIFT\",\"KEY_RIGHTSHIFT\",\"KEY_SPACE\"]"));
        let back: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(back.layout(), layout);
        // a hand-edited file with a typo keeps the rest of the layout
        let typo: Config = serde_json::from_str(r#"{"layout":{"Kick":["KEY_NOPE","KEY_Z"]}}"#).unwrap();
        assert_eq!(typo.layout().0[5], vec![K::KEY_Z]);
        assert_eq!(typo.layout().0[4], vec![K::KEY_J, K::KEY_U]);
    }

    #[test]
    fn key_names_read_well() {
        assert_eq!(key_name(K::KEY_J), "J");
        assert_eq!(key_name(K::KEY_LEFTSHIFT), "Left Shift");
        assert_eq!(key_name(K::KEY_BACKSPACE), "Backspace");
        assert_eq!(key_name(K::KEY_PAGEUP), "Page Up");
        assert_eq!(key_name(K::KEY_F12), "F12");
    }
}
