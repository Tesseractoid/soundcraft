#!/bin/bash
# Starts SoundCraft (with its control port) and the Console 1 / Fader controller.
# Part of the SoundCraft master repo (console1/ folder). Works on the laptop and the Mac Studio.
cd "$(dirname "$0")"
clear
if pgrep -f "bridge/main.py" >/dev/null || pgrep -f pt_adaptor.py >/dev/null; then
  echo "FaderBridge is running, and it would fight this for the Fader."
  echo "Quit it first (FaderBridge app or panel > Stop), then run this again."
  read -p "Press Enter to close."; exit 0
fi
if pgrep -f Console1OSD_APP >/dev/null; then
  echo "Softube's Console 1 display (OSD) is open, and it would fight this for the hardware."
  echo "Quit it first (menu bar icon > Quit), then run this again."
  read -p "Press Enter to close."; exit 0
fi
n=$(pgrep -x SoundCraft | wc -l | tr -d " ")
n2=$(pgrep -x soundcraft | wc -l | tr -d " ")
if [ $((n + n2)) -gt 1 ]; then
  echo "There are $((n + n2)) copies of SoundCraft running. Quit them all (Cmd-Q in each), then run this again."
  read -p "Press Enter to close."; exit 0
fi
OURS="$PWD/../target/release/soundcraft"      # the master build (repo root), with live meters
if [ -x "$OURS" ]; then
  xattr -d com.apple.quarantine "$OURS" 2>/dev/null       # copied from the laptop: let it run
  SC="$OURS"
else
  SC=/Applications/SoundCraft.app/Contents/MacOS/SoundCraft
fi
if ! nc -z 127.0.0.1 7801 2>/dev/null; then
  if pgrep -xi soundcraft >/dev/null; then
    echo "SoundCraft is open, but without its control port, so the controller can't talk to it."
    read -p "Quit SoundCraft and reopen it with the port? Save your work first. (y/n) " a
    [ "$a" = "y" ] || exit 0
    osascript -e 'quit app "SoundCraft"'; sleep 3
  fi
  echo "Starting SoundCraft with its control port (demo session)..."
  nohup "$SC" --demo --control 7801 >/dev/null 2>&1 &
  disown
fi
if pgrep -f console1_soundcraft.py >/dev/null; then
  echo "Stopping the controller that was already running..."
  pkill -f console1_soundcraft.py; sleep 1
fi
PY=.venv/bin/python
if ! $PY -c "import mido, rtmidi" 2>/dev/null; then PY="$HOME/.c1fader-venv/bin/python"; fi
(sleep 3; open http://127.0.0.1:7880) &
"$PY" -u console1_soundcraft.py 2>&1 | tee controller.log
echo
read -p "Press Enter to close."
