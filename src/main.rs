//! couch-strive: two keyboards, one Guilty Gear -Strive- local versus on Linux.
//! With "Keyboard Control Type" set to two players, GGST gives Controller 2 its own keys (arrows,
//! Insert, Delete, ...). couch-strive captures only Player 2's keyboard and replays it as those keys
//! on a virtual keyboard, so both players use their own keyboard. Player 1's keyboard is left alone.
//! Linux take on KeyboardSplitterXbox: an evdev grab replaces the Interception driver.
//!
//! `couch-strive` opens the window; the window runs `couch-strive helper` as root through pkexec.

mod gui;
mod helper;
mod layout;
mod protocol;

fn main() {
    let result = if std::env::args().nth(1).as_deref() == Some("helper") {
        helper::run()
    } else {
        gui::run().map_err(|e| std::io::Error::other(e.to_string()))
    };
    if let Err(e) = result {
        eprintln!("couch-strive: {e}");
        std::process::exit(1);
    }
}
