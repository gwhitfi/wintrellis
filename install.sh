#!/usr/bin/env bash
# Builds the release .exe and installs it to %LOCALAPPDATA%\Programs\WinTrellis\,
# then starts it. Run from WSL: ./install.sh
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
exe=target/x86_64-pc-windows-gnu/release/wintrellis.exe

# Windows won't overwrite a running .exe, and quitting normally runs the cleanup
# (unhides windows), so ask instead of killing it.
if tasklist.exe /FI "IMAGENAME eq wintrellis.exe" /NH | grep -qi wintrellis.exe; then
    echo "WinTrellis is running. Quit it (Win+Esc or the tray menu), then run this again." >&2
    exit 1
fi

# cmd.exe warns when started from a WSL path, so run it from C:\.
win_dir="$(cd /mnt/c && cmd.exe /c 'echo %LOCALAPPDATA%\Programs\WinTrellis' | tr -d '\r')"
dir="$(wslpath -u "$win_dir")"
mkdir -p "$dir"
cp "$exe" "$dir/wintrellis.exe"
echo "Installed to $win_dir"

# `start` launches it as a normal Windows program, not tied to this WSL shell.
(cd /mnt/c && cmd.exe /c start "" "$win_dir\\wintrellis.exe")
