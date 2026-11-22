#!/bin/bash
# Started by "SoundCraft (Console 1).app". While SoundCraft is open, it runs the Console 1
# controller whenever the hardware is plugged in (and starts it again if it's re-plugged).
# Quitting SoundCraft stops the controller. Log: console1/controller.log
PARENT="$1"
cd "$(dirname "$0")" || exit 1
PY=.venv/bin/python
[ -x "$PY" ] || PY="$HOME/.c1fader-venv/bin/python"
LOG=controller.log
CPID=""; STARTED=0; WAIT=4

present() { "$PY" -c "import mido,sys; sys.exit(0 if any('Console 1' in n for n in mido.get_output_names()) else 1)" 2>/dev/null; }
others()  { pgrep -f "bridge/main.py" >/dev/null || pgrep -f pt_adaptor.py >/dev/null || pgrep -f Console1OSD_APP >/dev/null; }

# A controller left over from an earlier SoundCraft session would keep running old settings:
# stop it so a fresh one starts with the current map.
if pgrep -f console1_soundcraft.py >/dev/null; then
  pkill -TERM -f console1_soundcraft.py; sleep 2
  pkill -KILL -f console1_soundcraft.py 2>/dev/null || true
fi
sleep 3   # give SoundCraft a moment to open its control port
while kill -0 "$PARENT" 2>/dev/null; do
  if [ -n "$CPID" ] && ! kill -0 "$CPID" 2>/dev/null; then
    # It stopped (usually the hardware was unplugged). If it died straight away, back off.
    [ $(( $(date +%s) - STARTED )) -lt 15 ] && WAIT=30 || WAIT=4
    CPID=""
  fi
  if [ -z "$CPID" ] && ! pgrep -f console1_soundcraft.py >/dev/null && ! others && present; then
    echo "=== $(date '+%F %T') autostart: Console 1 found, starting the controller" >> "$LOG"
    "$PY" -u console1_soundcraft.py >> "$LOG" 2>&1 &
    CPID=$!; STARTED=$(date +%s)
    osascript -e 'display notification "Console 1 connected" with title "SoundCraft"' >/dev/null 2>&1
  fi
  sleep "$WAIT"; WAIT=4
done
# SoundCraft has quit: stop our controller tidily (it turns the LEDs off on TERM).
# (Background jobs ignore Ctrl-C/INT, which is why this is TERM.)
if [ -n "$CPID" ]; then
  kill -TERM "$CPID" 2>/dev/null
  sleep 2; kill -KILL "$CPID" 2>/dev/null
fi
exit 0
