#!/bin/sh
# Installs couch-strive for the current user: the program, its icon and an app menu entry.
# Works from an unpacked release (program next to this script) or a built source checkout.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
bin="$here/couch-strive"
[ -f "$bin" ] || bin="$here/target/release/couch-strive"
[ -f "$bin" ] || { echo "couch-strive not found; in a source checkout run: cargo build --release" >&2; exit 1; }

prefix="$HOME/.local"
data="${XDG_DATA_HOME:-$prefix/share}"
install -Dm755 "$bin" "$prefix/bin/couch-strive"
install -Dm644 "$here/assets/couch-strive.svg" "$data/icons/hicolor/scalable/apps/couch-strive.svg"
mkdir -p "$data/applications"
# An absolute path works even when the desktop session's PATH lacks ~/.local/bin.
sed "s|^Exec=.*|Exec=$prefix/bin/couch-strive|" "$here/assets/couch-strive.desktop" > "$data/applications/couch-strive.desktop"

echo "Installed couch-strive to $prefix/bin. Open it from your app menu."
