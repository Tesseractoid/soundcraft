#!/usr/bin/env python3
"""Console 1 hardware mapper.

Move one control at a time. When the script sees a control it hasn't
met yet, it asks you to type the label printed on the panel.
Results are saved to console1_map.json after every control, and every
raw MIDI message is logged to raw_log.jsonl.
"""
import json, sys, time
from pathlib import Path
import mido

HERE = Path(__file__).resolve().parent
MAP_FILE = HERE / "console1_map.json"
RAW_FILE = HERE / "raw_log.jsonl"
SAMPLE_SECONDS = 0.8  # how long to watch a new control before asking


def pick_port(names, kind):
    if not names:
        sys.exit(f"No MIDI {kind} ports found. Is the Console 1 plugged in?")
    hits = [n for n in names if "console 1" in n.lower() and "fader" not in n.lower()]
    if len(hits) == 1:
        return hits[0]
    print(f"\nMIDI {kind} ports:")
    for i, n in enumerate(names):
        print(f"  {i}: {n}")
    return names[int(input(f"Which number is the Console 1 {kind}? "))]


def key_of(msg):
    if msg.type == "control_change":
        return f"cc:{msg.channel}:{msg.control}"
    if msg.type in ("note_on", "note_off"):
        return f"note:{msg.channel}:{msg.note}"
    if msg.type == "pitchwheel":
        return f"pitch:{msg.channel}"
    if msg.type == "sysex":
        return "sysex"
    return f"{msg.type}:{getattr(msg, 'channel', '-')}"


def value_of(msg):
    for attr in ("value", "velocity", "pitch"):
        if hasattr(msg, attr):
            return getattr(msg, attr)
    return None


def guess_kind(key, values):
    if key.startswith("note"):
        return "button (note)"
    uniq = sorted(set(v for v in values if v is not None))
    if uniq and set(uniq) <= {0, 127}:
        return "button (cc 0/127)"
    if uniq and all(v < 16 or v > 111 for v in uniq):
        return "relative encoder (two's complement)"
    if uniq and all(48 <= v <= 80 for v in uniq) and 64 not in uniq:
        return "relative encoder (offset 64)"
    return "absolute knob"


def main():
    in_name = pick_port(mido.get_input_names(), "input")
    out_names = mido.get_output_names()
    out_name = next((n for n in out_names if n.lower() == in_name.lower()), None)
    print(f"\nListening on: {in_name}")
    print(f"Matching output (for LED tests later): {out_name or 'not found'}")

    mapping = json.loads(MAP_FILE.read_text()) if MAP_FILE.exists() else {
        "device_in": in_name, "device_out": out_name, "controls": {}}
    known = mapping["controls"]
    if known:
        print(f"Resuming: {len(known)} controls already mapped.")

    print("\nMove ONE control at a time (turn a knob a little, or press a button).")
    print("Type the label from the panel and press Enter.")
    print("Empty Enter (no label) = finished, save and quit.  Type - to skip a control.\n")

    raw = RAW_FILE.open("a")
    with mido.open_input(in_name) as port:
        while True:
            msg = port.receive()
            raw.write(json.dumps({"t": time.time(), "msg": str(msg)}) + "\n"); raw.flush()
            k = key_of(msg)
            if k in known or k == "sysex" and "sysex" in known:
                continue
            # watch the new control briefly to learn how it behaves
            values, msgs = [value_of(msg)], [str(msg)]
            end = time.time() + SAMPLE_SECONDS
            while time.time() < end:
                for m in port.iter_pending():
                    raw.write(json.dumps({"t": time.time(), "msg": str(m)}) + "\n")
                    if key_of(m) == k:
                        values.append(value_of(m)); msgs.append(str(m))
                time.sleep(0.01)
            kind = guess_kind(k, values)
            print(f"New control  {k}   values seen {sorted(set(values))[:12]}   -> looks like: {kind}")
            label = input("  Label on panel (empty Enter = finish): ").strip()
            for m in port.iter_pending():  # drop anything that arrived while you were typing
                raw.write(json.dumps({"t": time.time(), "msg": str(m), "dropped": True}) + "\n")
            if not label:
                break
            if label == "-":
                print("  skipped\n"); continue
            known[k] = {"label": label, "kind_guess": kind,
                        "values_seen": sorted(set(v for v in values if v is not None)),
                        "example": msgs[0]}
            MAP_FILE.write_text(json.dumps(mapping, indent=2))
            print(f"  saved ({len(known)} mapped)\n")
    raw.close()
    print(f"\nDone. {len(known)} controls saved to {MAP_FILE}")


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("\nStopped. Progress is saved; run again to carry on.")
