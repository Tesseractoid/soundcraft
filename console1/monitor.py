#!/usr/bin/env python3
"""Shows every message the Console 1 sends, with its mapped name. Saved to monitor_log.txt."""
import json, time
from pathlib import Path
import mido
HERE = Path(__file__).resolve().parent
m = json.loads((HERE / "console1_map.json").read_text())
names = {k: v["label"] for k, v in m["controls"].items()}
print("For each double-named button: press it once normally, then HOLD SHIFT and press it once. Go slowly, in a fixed order.")
print("Lines marked NOT IN MAP are new messages.")
print("Ctrl-C when done.\n")
with mido.open_input(m["device_in"]) as port, (HERE / "monitor_log.txt").open("w") as f:
    try:
        for msg in port:
            key = f"cc:{msg.channel}:{msg.control}" if msg.type == "control_change" else msg.type
            line = f"{time.strftime('%H:%M:%S')}  {msg}   -> {names.get(key, 'NOT IN MAP')}"
            print(line, flush=True); f.write(line + "\n"); f.flush()
    except KeyboardInterrupt:
        pass
print("\nSaved monitor_log.txt. Tell Claude.")
