# couch-strive

Play Guilty Gear -Strive- local versus on Linux with two keyboards, one per player.

GGST can give a second player their own set of keys on the keyboard ("Controller 2": arrows,
Insert, Delete, Home, End, Page Up). couch-strive captures only Player 2's keyboard and replays it
through a virtual keyboard as those keys, so both players use the same layout on their own
keyboard. Player 1's keyboard is left alone.

Inspired by [KeyboardSplitterXbox](https://github.com/djlastnight/KeyboardSplitterXbox). On Linux,
an evdev grab replaces its Interception driver, so no drivers are needed.

## Install

Download the latest `couch-strive-*-linux-x86_64.tar.gz` from
[Releases](https://github.com/SHN2004/couch-strive/releases), unpack it and run the installer:

```bash
tar -xzf couch-strive-*-linux-x86_64.tar.gz
./couch-strive-*-linux-x86_64/install.sh
```

Or build from source:

```bash
cargo build --release
./install.sh
```

The installer puts the program in `~/.local/bin`, plus an icon and an app menu entry. To remove
it, delete `~/.local/bin/couch-strive`, `~/.local/share/applications/couch-strive.desktop` and
`~/.local/share/icons/hicolor/scalable/apps/couch-strive.svg`.

## Use

Open **couch-strive** from your app menu. Run it as yourself, not with sudo. In the window:

1. Click **Grant access**. Your system asks for your password, because reading keyboards needs
   admin rights.
2. If keyd holds the keyboards, click **Pause keyd while playing**. It starts again when you close
   couch-strive.
3. Pick **Player 2's keyboard**. The key tester shows what each key sends.
4. Click the green **Start** button. The status turns **PLAYING**; until then nothing reaches the game.

**F12** on Player 2's keyboard switches it between the game and the desktop. Closing the window
gives the keyboard back.

## One-time GGST setup

1. In Settings, set **Keyboard Control Type** to the two-player option.
2. In Button Settings, press **0** to open Controller 2, select **Dash**, and press Player 2's Dash
   key while couch-strive is running (it sends Page Down).

## Player 2 layout

Click **+** next to an action and press a key on Player 2's keyboard to add it, or click a key to
remove it. The layout and the chosen keyboard are saved in `~/.config/couch-strive/config.json`.

The default layout:

| Player 2 presses | couch-strive sends | GGST Controller 2 |
|---|---|---|
| WASD or arrows | Arrows | Movement |
| J or U | Insert | Punch, menu confirm |
| I | Delete | Kick |
| K | Home | Slash |
| L | End | Heavy Slash |
| O or Backspace | Page Up | Dust, menu back |
| Shift | Page Down | Dash (after the setup above) |

Player 1 plays with GGST's own keyboard controls and should avoid the keys in the middle column.

## How it works

The window runs as you and starts `couch-strive helper` as root through `pkexec`. The helper only
lists keyboards, reports key presses, captures Player 2's keyboard and types on the virtual
keyboard. They talk in JSON lines over the helper's stdin and stdout. When the window closes, the
helper releases the keyboard, starts keyd again if it paused it, and exits.

## Releasing

Set the new version in `Cargo.toml`, commit, then push a matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions builds the program on Ubuntu 22.04 (so it runs on older distros too) and publishes
the release with the download above.

## Limits

- Two keyboards on one wireless receiver look like one keyboard to couch-strive.
- Tested with GGST through Heroic and GE-Proton on CachyOS.
