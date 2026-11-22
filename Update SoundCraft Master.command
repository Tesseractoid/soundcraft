#!/bin/bash
# Double-click after pulling in GitHub Desktop. Builds everything in this repo for THIS Mac:
#   1. SoundCraft (master: Console 1 meters + automation fixes)  -> ~/Applications/SoundCraft (Console 1).app
#   2. AK 4K EQ plug-in (CLAP + VST3)                             -> ~/Library/Audio/Plug-Ins
#   3. The Console 1 controller's Python packages                 -> console1/.venv
# A full log goes to update-log.txt in this folder.
cd "$(dirname "$0")" || exit 1
ROOT="$PWD"
export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
LOG="$ROOT/update-log.txt"
: > "$LOG"
clear
say() { echo "$1"; echo "=== $(date +%T) $1" >> "$LOG"; }
run() {  # run quietly with progress dots, everything into the log
  "$@" >> "$LOG" 2>&1 &
  local pid=$!
  while kill -0 $pid 2>/dev/null; do printf "."; sleep 3; done
  wait $pid; local s=$?; echo; return $s
}
fail() { say "FAILED: $1"; echo; echo "Something went wrong. Tell Claude - it can read update-log.txt in this folder."; read -n 1 -s -r -p "Press any key to close."; exit 1; }

say "SoundCraft master - updating this Mac ($(uname -m)). Branch: $(git branch --show-current 2>/dev/null), commit $(git rev-parse --short HEAD 2>/dev/null)"
{ rustc --version; cargo --version; } >> "$LOG" 2>&1 || fail "Rust isn't installed on this Mac"

say "1/3 Building SoundCraft (first time on a Mac: 10-20 minutes)"
run cargo build --release -p soundcraft || fail "SoundCraft build"
run bash console1/make_app.sh || fail "making the SoundCraft (Console 1) app"
say "    -> ~/Applications/SoundCraft (Console 1).app"

say "2/3 Building AK 4K EQ"
# The plug-in's bundler always uses the top-most Cargo.toml it can find, which inside this repo
# is SoundCraft's. So it's built from a copy outside the repo (kept, so rebuilds stay quick).
EQB="$HOME/Library/Caches/ak-4k-eq-build"
mkdir -p "$EQB"
rsync -a --delete --exclude target --exclude build-log.txt plugins/ak-4k-eq/ "$EQB/" || fail "copying AK 4K EQ"
( cd "$EQB" && run cargo xtask bundle ak_4k_eq --release ) || fail "AK 4K EQ build"
mkdir -p "$HOME/Library/Audio/Plug-Ins/CLAP" "$HOME/Library/Audio/Plug-Ins/VST3"
rm -rf "$HOME/Library/Audio/Plug-Ins/CLAP/AK 4K EQ.clap" "$HOME/Library/Audio/Plug-Ins/VST3/AK 4K EQ.vst3"
cp -R "$EQB/target/bundled/AK 4K EQ.clap" "$HOME/Library/Audio/Plug-Ins/CLAP/" || fail "installing AK 4K EQ"
cp -R "$EQB/target/bundled/AK 4K EQ.vst3" "$HOME/Library/Audio/Plug-Ins/VST3/" || fail "installing AK 4K EQ"
say "    -> installed (CLAP + VST3)"

say "3/3 Console 1 controller packages"
# Uses Apple's own Python (the one the controller has always used); a newer Homebrew Python
# can't always set itself up. A half-made .venv is rebuilt.
if ! console1/.venv/bin/python -m pip --version >/dev/null 2>&1; then
  run /usr/bin/python3 -m venv --clear console1/.venv || fail "making console1/.venv"
fi
console1/.venv/bin/python -c "import mido, rtmidi" 2>/dev/null \
  || run console1/.venv/bin/python -m pip install -q mido python-rtmidi || fail "installing mido / python-rtmidi"

say "ALL DONE"
osascript -e 'display notification "SoundCraft master and AK 4K EQ are up to date." with title "SoundCraft master"' >/dev/null 2>&1
echo
echo "Open 'SoundCraft (Console 1)' from your Applications folder (in your home folder),"
echo "or double-click console1/Start Controller to start it with the Console 1."
read -n 1 -s -r -p "Press any key to close."
