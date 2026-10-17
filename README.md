# couch-strive

Play Guilty Gear -Strive- local versus on Linux with two keyboards, one per player.

GGST can give a second player their own set of keys on the keyboard ("Controller 2": arrows,
Insert, Delete, Home, End, Page Up). couch-strive captures only Player 2's keyboard and replays it
through a virtual keyboard as those keys, so both players use the same layout on their own
keyboard. Player 1's keyboard is left alone.

Inspired by [KeyboardSplitterXbox](https://github.com/djlastnight/KeyboardSplitterXbox). On Linux,
an evdev grab replaces its Interception driver, so no drivers are needed.

## Build and run

```bash
cargo build --release
sudo ./target/release/couch-strive
```

Player 2 then presses Space on their keyboard. couch-strive needs root (or the `input` group) to read
keyboards, and write access to `/dev/uinput`.

Stop other key remappers that grab keyboards first, for example `sudo systemctl stop keyd`.
couch-strive exits with a message if one is in the way.

## One-time GGST setup

1. In Settings, set **Keyboard Control Type** to the two-player option.
2. In Button Settings, press **0** to open Controller 2, select **Dash**, and press Shift on
   Player 2's keyboard while couch-strive is running (it sends Page Down).

## Player 2 layout

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

**F12** on Player 2's keyboard switches it between the game and the desktop. **Ctrl+C** in the
terminal quits.

## Limits

- Two keyboards on one wireless receiver look like one keyboard to couch-strive.
- Tested with GGST through Heroic and GE-Proton on CachyOS.
