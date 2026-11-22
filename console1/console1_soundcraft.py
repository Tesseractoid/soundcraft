#!/usr/bin/env python3
"""Console 1 -> SoundCraft controller, version 1.

Hardware map: console1_map.json   (made by map_console1.py)
Layout:       strip_layout.json   (what each control does)
Needs SoundCraft running with its control port: SoundCraft --control 7801
"""
import json, math, socket, sys, threading, time, collections
from http.server import ThreadingHTTPServer, BaseHTTPRequestHandler
from pathlib import Path
import mido

HERE = Path(__file__).resolve().parent
HW = json.loads((HERE / "console1_map.json").read_text())
LAYOUT = json.loads((HERE / "strip_layout.json").read_text())
SECTIONS = LAYOUT["sections"]
SEND = LAYOUT.get("send_layer", {})
SEND_LAYER = {}
for _i, (_lvl, _pan) in enumerate(SEND.get("knobs", [])):
    SEND_LAYER[_lvl] = {"send": _i, "what": "level"}
    SEND_LAYER[_pan] = {"send": _i, "what": "pan"}
POLL_S, FLUSH_S, TOUCH_HOLD_S = 0.2, 0.02, 0.6


LOG_TAIL = collections.deque(maxlen=12)


SOFTUBE = LAYOUT.get("hardware_mode", "softube") == "softube"
if SOFTUBE:
    from c1_softube import Console1Softube


def log(*a):
    line = " ".join(str(x) for x in a)
    print(time.strftime("%H:%M:%S"), line, flush=True)
    LOG_TAIL.append(time.strftime("%H:%M:%S") + "  " + line)


# ---------- hardware ----------
label_to_cc, cc_to_label = {}, {}
for key, info in HW["controls"].items():
    if key.startswith("cc:"):
        _, ch, num = key.split(":")
        label_to_cc[info["label"]] = (int(ch), int(num))
        cc_to_label[(int(ch), int(num))] = info["label"]

missing = [l for l in list(LAYOUT["knobs"]) + list(LAYOUT["buttons"]) + LAYOUT["track_buttons"] if l not in label_to_cc]
if missing:
    log("Note: these layout controls aren't in the hardware map and will be ignored:", ", ".join(missing))


# ---------- SoundCraft control port ----------
class SoundCraft:
    def __init__(self, port):
        self.port, self.lock, self.next_id, self.f = port, threading.Lock(), 1, None
        self.last_error = None
        self.connected = False

    def connect(self):
        while True:
            try:
                s = socket.create_connection(("127.0.0.1", self.port), timeout=10)
                self.f = s.makefile("rw")
                self.connected = True
                log(f"Connected to SoundCraft on port {self.port}")
                return
            except OSError:
                self.connected = False
                log(f"Waiting for SoundCraft on port {self.port} (start it with --control {self.port})...")
                time.sleep(2)

    def call(self, method, params=None):
        with self.lock:
            try:
                rid = self.next_id; self.next_id += 1
                self.f.write(json.dumps({"id": rid, "method": method, "params": params or {}}) + "\n")
                self.f.flush()
                reply = json.loads(self.f.readline())
            except (OSError, ValueError, AttributeError):
                log("Lost connection to SoundCraft; reconnecting...")
                self.connect()
                return None
        if not reply.get("ok"):
            err = reply.get("error")
            if err != self.last_error:
                log("SoundCraft said:", err)
                self.last_error = err
            return None
        return reply.get("result")

    def cmd(self, command, **params):
        return self.call("engine.execute", {"command": command, "params": params})


# ---------- value conversion ----------
def knob_to_value(spec, k):
    t = k / 127.0
    lo, hi = spec["min"], spec["max"]
    if spec.get("taper") == "log":
        v = lo * (hi / lo) ** t
    else:
        v = lo + (hi - lo) * t
    if spec.get("detent") and abs(k - 64) <= 1:
        v = (lo + hi) / 2.0
    return v


def value_to_knob(spec, v):
    lo, hi = spec["min"], spec["max"]
    v = min(max(v, min(lo, hi)), max(lo, hi))   # min may be above max for reversed knobs
    t = math.log(v / lo) / math.log(hi / lo) if spec.get("taper") == "log" else (v - lo) / (hi - lo)
    return int(round(t * 127))


# ---------- CLAP plug-ins (AK 4K EQ) ----------
# A hosted CLAP plug-in's parameters are addressed by number and take 0..1 values. A section with
# "clap_params" maps our own parameter names and real units (dB, Hz) to and from those.
def _clap(section):
    return SECTIONS.get(section, {}).get("clap_params")


def _to_norm(m, v):
    if m.get("bool"):
        return 1.0 if v >= 0.5 else 0.0
    lo, hi = m["min"], m["max"]
    return ((min(max(v, lo), hi) - lo) / (hi - lo)) ** (2.0 ** m.get("skew", 0.0))


def _from_norm(m, n):
    if m.get("bool"):
        return 1.0 if n >= 0.5 else 0.0
    n = min(max(n, 0.0), 1.0)
    return m["min"] + (m["max"] - m["min"]) * n ** (1.0 / 2.0 ** m.get("skew", 0.0))


def to_plugin(section, params):
    cp = _clap(section)
    if not cp:
        return params
    return {(str(cp[k]["id"]) if k in cp else k): (_to_norm(cp[k], v) if k in cp else v) for k, v in params.items()}


def from_plugin(section, params):
    cp = _clap(section)
    if not cp:
        return params
    out = dict(params)
    for name, m in cp.items():
        raw = params.get(str(m["id"]))
        out[name] = _from_norm(m, raw) if raw is not None else m.get("default")
    return out


def knob_to_db(k):          # fader-style curve: 0 = -inf, ~96 = 0 dB, 127 = +12 dB
    return -144.0 if k == 0 else max(-144.0, 12 + 96 * math.log10(k / 127.0))


def db_to_knob(db):
    return 0 if db <= -140 else int(round(min(127, 127 * 10 ** ((db - 12) / 96))))


# ---------- controller ----------
class Controller:
    def __init__(self):
        self.sc = SoundCraft(LAYOUT.get("soundcraft_port", 7801))
        self.sc_meters = SoundCraft(LAYOUT.get("soundcraft_port", 7801))   # meters get their own line, so they never queue behind knobs
        self.out = None if SOFTUBE else mido.open_output(HW.get("device_out") or "Console 1")
        self.c1 = None                                   # Softube-mode hardware (opened in run)
        self.fader = None                                # Fader mk1 (opened in run, if plugged in)
        self.LAYOUT_KNOBS = LAYOUT["knobs"]
        self.knob_val = {}                               # label -> value we're turning, while being turned
        self.state_lock = threading.Lock()
        self.tracks, self.selected, self.bank = [], None, 0
        self.selection = []        # every selected track id; the first is the one shown on the hardware
        self.group_mode = False    # Group on: track buttons add/remove tracks instead of replacing
        self.vol_base, self.touched_vol = {}, {}         # primary track -> last volume we set, for group volume moves
        self.pending = {}          # label -> latest knob value waiting to be sent
        self.touched = {}          # label -> time last moved by hand
        self.led = {}              # (ch, cc) -> value last sent
        self.phase, self.trim = {}, {}
        self.width, self.pan_last = {}, {}
        self.busses = []
        self.meters_unavailable, self.meter_t, self.meter_level, self.meter_sent, self.meter_row = False, time.time(), {}, {}, None
        self.meter_dark = {}       # section meters switched off (no plug-in / inactive)
        self.track_db = {}         # per-track output level (dB), drives the track button lights
        self.layer = False         # send layer on?
        self.fb_lock = threading.Lock()            # stereo track -> half-width of its image, remembered while panning   # not reported by SoundCraft yet; kept locally
        self.insert_wait = {}
        self.logged = {}
        self.last_nudge, self.win, self.win_known = 0, (800, 600), False
        self.popup = None          # [track, slot, opened_by_us, last_touch] for the plug-in window being adjusted      # (track, section) -> time we asked for an insert

    # --- state from SoundCraft ---
    def poll(self):
        s = self.sc.call("session.inspect", {"detail": "full"})
        if not s:
            return
        with self.state_lock:
            self.tracks = [t for t in s.get("tracks", []) if not t.get("hidden")]
            self.busses = sorted(s.get("busses") or [], key=lambda b: b.get("id", 0))
            sel = (s.get("selection") or {}).get("tracks") or []
            self.selection = list(sel)
            if self.selected not in sel:
                self.selected = sel[0] if sel else None

    def nudge(self):
        """SoundCraft's window only redraws on its own input, so poke it after changes."""
        mode = LAYOUT.get("repaint_nudge", "move")
        if mode == "none" or time.time() - self.last_nudge < 0.05:
            return
        self.last_nudge = time.time()
        if mode == "move":
            w, h = self.win
            self.sc.call("ui.move", {"x": 4, "y": max(4, h - 4)})
        elif mode == "inspect":
            self.sc.call("ui.inspect")

    def plugin_windows(self):
        r = self.sc.call("ui.inspect") or {}
        return [list(w) for w in (r.get("ui") or {}).get("plugin_windows") or []]

    def show_plugin(self, tid, slot):
        cfg = LAYOUT.get("plugin_popup", {})
        if not cfg.get("enabled", True):
            return
        now = time.time()
        if self.popup and self.popup[:2] == [tid, slot]:
            self.popup[3] = now
            return
        self.hide_plugin()
        wins = self.plugin_windows()
        key = [tid, slot]
        opened = key not in wins
        wins = [w for w in wins if w != key] + [key]       # last in the list = in front
        self.sc.call("ui.set", {"plugin_windows": wins})
        self.popup = [tid, slot, opened, now]

    def hide_plugin(self):
        if not self.popup:
            return
        tid, slot, opened, _ = self.popup
        self.popup = None
        if opened:                                          # only close windows we opened
            wins = [w for w in self.plugin_windows() if w != [tid, slot]]
            self.sc.call("ui.set", {"plugin_windows": wins})

    def automated(self, t, param):
        """True when SoundCraft is playing back automation for this control, so knob moves won't stick."""
        if (t.get("automation_mode") or "off") == "off":
            return False
        a = t.get("automation") or {}
        names = a.keys() if isinstance(a, dict) else [x.get("param") or x.get("target") for x in a if isinstance(x, dict)]
        return param in names

    def track(self):
        return next((t for t in self.tracks if t["id"] == self.selected), None)

    def targets(self):
        """Every selected track, the shown one first. A multi-track selection is the temporary group."""
        t = self.track()
        if not t:
            return []
        others = [tr for tr in self.tracks if tr["id"] in self.selection and tr["id"] != t["id"]]
        return [t] + others

    def section_insert(self, t, section):
        sec = SECTIONS[section]
        for ins in t.get("inserts") or []:   # use the track's own plug-in of this type, wherever it is
            if ins and ins.get("plugin") == sec["plugin"]:
                if sec.get("clap_params"):
                    ins = dict(ins, params=from_plugin(section, ins.get("params") or {}))
                return ins
        return None

    def ensure_section(self, t, section, first_params=None):
        if self.section_insert(t, section):
            return True
        key = (t["id"], section)
        if time.time() - self.insert_wait.get(key, 0) < 2:
            return True    # already asked; the next poll will show it
        sec = SECTIONS[section]
        params = dict(sec.get("insert_params", {}))
        params.update(first_params or {})
        used = {i.get("slot") for i in (t.get("inserts") or []) if i}
        allowed = range(LAYOUT.get("max_slots", 5))          # inserts A-E only
        pref = sec.get("slot")
        free = pref if pref in allowed and pref not in used else next((n for n in allowed if n not in used), None)
        if free is None:
            log(f"{t['name']}: inserts A-E are full, so {sec['plugin']} wasn't added")
            return False
        r = self.sc.cmd("mix.insert", track=t["id"], slot=free, plugin=sec["plugin"], params=to_plugin(section, params))
        self.insert_wait[key] = time.time()
        if r is not None:
            log(f"Added {sec['plugin']} to {t['name']} (insert {chr(97 + free).upper()})")
        return False

    # --- hardware input ---
    def knob_k(self, t, label, spec):
        """Where a knob's ring should sit (0..127) for the selected track, from SoundCraft's state."""
        k = 0
        if not t:
            return k
        target = spec.get("target")
        if target == "volume":
            k = db_to_knob(t.get("volume_db", 0.0))
        elif target == "pan":
            pans = t.get("pan") or [0.0]
            if len(pans) >= 2:
                k = int(round(127 * max(0.0, min(1.0, (pans[1] - pans[0]) / 2.0))))
            else:
                k = int(round(64 + pans[0] * 63))
        elif target == "trim":
            k = value_to_knob(spec, t.get("trim_db", self.trim.get(t["id"], 0.0)))
        elif "section" in spec:
            ins = self.section_insert(t, spec["section"])
            if ins:
                params = ins.get("params", {})
                if "switch" in spec and not params.get(spec["switch"]):
                    k = 0 if spec.get("off_at") == "min" else 127
                elif spec["param"] in params:
                    k = value_to_knob(spec, params[spec["param"]])
            elif "switch" in spec:
                k = 0 if spec.get("off_at") == "min" else 127
            elif "default" in spec:      # plug-in not on the track yet: start where the plug-in itself starts (EQ gains at 0 dB)
                k = value_to_knob(spec, spec["default"])
        return k

    def button_on(self, t, label):
        """Is this toggle on, according to SoundCraft?"""
        spec = LAYOUT["buttons"].get(label)
        if not spec or not t:
            return False
        target = spec["target"]
        if target in ("mute", "solo"):
            return bool(t.get(target))
        if target == "phase":
            return bool(t.get("phase_invert", self.phase.get(t["id"], False)))
        if target == "section":
            ins = self.section_insert(t, spec["section"])
            return bool(ins and ins.get("active", True))
        if target == "param_toggle":
            ins = self.section_insert(t, spec["section"])
            return bool(ins and ins.get("params", {}).get(spec["param"]) == spec["on"])
        return False

    def on_event(self, label, value, pressed):
        """Softube mode: knobs arrive as clicks (+1/-1), buttons as press/release."""
        if label == "Shift":
            return
        if label in LAYOUT["knobs"]:
            now = time.time()
            with self.state_lock:
                t = self.track()
                base = self.knob_val.get(label)
                if base is None or now - self.touched.get(label, 0) > TOUCH_HOLD_S:
                    lspec = SEND_LAYER.get(label) if self.layer else None
                    if lspec and t:
                        snd = self.send_of(t, lspec["send"])
                        base = (db_to_knob(snd.get("level_db", -144)) if lspec["what"] == "level" else int(round(64 + (snd.get("pan") or 0.0) * 63))) if snd else 0
                    else:
                        base = self.knob_k(t, label, LAYOUT["knobs"][label])
                step = LAYOUT.get("knob_step", 2.0) * (LAYOUT.get("fine_factor", 0.25) if self.c1 and self.c1.shift else 1.0)
                new = max(0.0, min(127.0, base + value * step))
                self.knob_val[label] = new
                self.pending[label] = int(round(new))
                self.touched[label] = now
            if self.c1:
                self.c1.ring(label, new)              # the ring follows the knob straight away
                self.c1.light_enable(label, True)     # a dark ring lights as soon as you turn it
                self.led[label] = int(round(new))
            return
        if not pressed:
            return                                    # act on press; releases matter for Shift/holds later
        shift = bool(self.c1 and self.c1.shift)
        log(f"Button: {'Shift + ' if shift else ''}{label}")
        if SEND.get("shift") and label == SEND.get("button") and shift:
            self.layer = not self.layer           # Shift + Equalizer flips the EQ section to sends
            log("Send layer ON (EQ gains = sends A-D, EQ freqs = send pans)" if self.layer else "Send layer off")
            threading.Thread(target=self.feedback, kwargs={"force": True}, daemon=True).start()
            return
        with self.state_lock:
            t = self.track()
        on = not self.button_on(t, label)
        threading.Thread(target=self.press, args=(label, on), daemon=True).start()

    def on_midi(self, msg):
        if msg.type != "control_change":
            return
        label = cc_to_label.get((msg.channel, msg.control))
        if not label:
            return
        if label in LAYOUT["knobs"]:
            with self.state_lock:
                self.pending[label] = msg.value
                self.touched[label] = time.time()
        else:   # the buttons latch in hardware: 127 = turned on, 0 = turned off
            log(f"Button: {label} {'on' if msg.value else 'off'}")
            threading.Thread(target=self.press, args=(label, msg.value > 0), daemon=True).start()

    def flush(self):
        with self.state_lock:
            todo, self.pending = self.pending, {}
            t = self.track()
            group = self.targets()
        if not todo:
            return
        if not t:
            log("No track selected in SoundCraft — select one (or press a track button).")
            return
        try:
            self.win = tuple((self.sc.call("ui.inspect") or {}).get("window_size") or self.win) if not self.win_known else self.win
            self.win_known = True
        except Exception:
            pass
        for label, k in todo.items():
            lspec = SEND_LAYER.get(label) if self.layer else None
            if lspec:
                for g in group:
                    self.turn_send(g, lspec, k)
                continue
            if time.time() - self.logged.get(label, 0) > 0.5:
                who = t["name"] + (f" + {len(group) - 1}" if len(group) > 1 else "")
                log(f"{label} = {k} on {who}"); self.logged[label] = time.time()
            spec = LAYOUT["knobs"][label]
            if spec.get("target") == "volume":
                self.turn_volume(group, k)
            else:
                for g in group:
                    self.turn(g, label, spec, k, show=(g is t))
        self.nudge()

    def turn_volume(self, group, k):
        """Volume: the shown track goes to the knob's level; the rest of the group moves by the same dB (keeps the balance)."""
        t = group[0]
        target = knob_to_db(k)
        if len(group) == 1:
            self.sc.cmd("mix.volume", tracks=[t["id"]], db=round(target, 2))
            return
        base = self.vol_base.get(t["id"])
        if base is None or time.time() - self.touched_vol.get(t["id"], 0) > 1.0:
            base = t.get("volume_db", 0.0)
        self.touched_vol[t["id"]] = time.time()
        delta = target - base
        if abs(delta) > 0.001:
            self.sc.cmd("mix.volume", tracks=[g["id"] for g in group], delta_db=round(delta, 2))
        self.vol_base[t["id"]] = target

    def turn(self, t, label, spec, k, show=True):
        tid, target = t["id"], spec.get("target")
        if target == "pan":
            c = 0.0 if abs(k - 64) <= 1 else max(-1.0, min(1.0, (k - 64) / 63.0))
            pans = t.get("pan") or [0.0]
            if len(pans) < 2:
                self.sc.cmd("mix.pan", tracks=[tid], pan=c)
            else:
                # stereo: the knob is width. Fully left = mono (both pans centre), fully right = full stereo.
                w = k / 127.0
                self.sc.cmd("mix.pan", tracks=[tid], pan=round(-w, 4), index=0)
                self.sc.cmd("mix.pan", tracks=[tid], pan=round(w, 4), index=1)
        elif target == "trim":
            db = knob_to_value(spec, k); self.trim[tid] = db
            self.sc.cmd("mix.trim", tracks=[tid], db=round(db, 2))
        elif "section" in spec:
            value = knob_to_value(spec, k)
            params = {spec["param"]: value}
            if "switch" in spec:
                off = (k == 0) if spec.get("off_at") == "min" else (k == 127)
                params[spec["switch"]] = 0.0 if off else 1.0
            if not self.ensure_section(t, spec["section"], params):
                return
            ins = self.section_insert(t, spec["section"])
            if ins:
                self.sc.cmd("mix.insert_params", track=tid, slot=ins["slot"], params=to_plugin(spec["section"], params))
                if show:
                    self.show_plugin(tid, ins["slot"])

    def send_of(self, t, slot):
        return next((x for x in t.get("sends") or [] if x and x.get("slot") == slot), None)

    def turn_send(self, t, lspec, k):
        slot, tid = lspec["send"], t["id"]
        snd = self.send_of(t, slot)
        if lspec["what"] == "level":
            db = round(knob_to_db(k), 2)
            if snd:
                self.sc.cmd("mix.send_level", track=tid, slot=slot, db=db)
            elif slot < len(self.busses):
                if time.time() - self.insert_wait.get((tid, "send", slot), 0) > 2:
                    bus = self.busses[slot]["name"]
                    self.sc.cmd("mix.send", track=tid, slot=slot, bus=bus, level_db=db)
                    self.insert_wait[(tid, "send", slot)] = time.time()
                    log(f"Added send {chr(65 + slot)} on {t['name']} to bus {bus}")
            elif time.time() - self.logged.get(("nobus", slot), 0) > 3:
                self.logged[("nobus", slot)] = time.time()
                log(f"No bus for send {chr(65 + slot)} yet: add a bus in SoundCraft (it becomes bus {slot + 1})")
        elif snd:
            pan = 0.0 if abs(k - 64) <= 1 else max(-1.0, min(1.0, (k - 64) / 63.0))
            self.sc.cmd("mix.send_level", track=tid, slot=slot, pan=round(pan, 4))

    def press(self, label, on=True):
        with self.state_lock:
            t = self.track()
            tracks = list(self.tracks)
            group = self.targets()
            selection = list(self.selection)
        if label in LAYOUT["track_buttons"]:
            i = self.bank * 20 + LAYOUT["track_buttons"].index(label)
            if i < len(tracks):
                tid = tracks[i]["id"]
                if self.group_mode:
                    if tid in selection and len(selection) > 1:
                        selection.remove(tid)
                        log(f"Group: removed {tracks[i]['name']}")
                    elif tid not in selection:
                        selection.append(tid)
                        log(f"Group: added {tracks[i]['name']}")
                    if self.selected not in selection:
                        self.selected = selection[0]
                    self.sc.cmd("edit.select", tracks=selection)
                else:
                    self.selected = tid
                    self.sc.cmd("edit.select", tracks=[tid])
                    log(f"Selected {tracks[i]['name']}")
            self.led.pop(label_to_cc.get(label), None)
            self.poll(); self.feedback(force=True)
            return
        if label == LAYOUT.get("group_button"):
            self.group_mode = not self.group_mode
            if self.group_mode:
                log("Group ON: tap track buttons to add or remove tracks; everything you touch applies to all of them")
            else:
                if t and len(selection) > 1:
                    self.sc.cmd("edit.select", tracks=[t["id"]])
                log("Group off: back to one track")
            self.led.pop(label_to_cc.get(label), None)
            self.poll(); self.feedback(force=True)
            return
        if label == SEND.get("button") and not SEND.get("shift"):
            self.layer = not self.layer          # every press flips it, whatever the hardware latch says
            self.led.pop(label_to_cc.get(label), None)
            log("Send layer ON (EQ gains = sends A-D, EQ freqs = send pans)" if self.layer else "Send layer off")
            self.feedback(force=True)
            return
        spec = LAYOUT["buttons"].get(label)
        if not spec:
            return
        target = spec["target"]
        if target == "bank":
            self.bank = max(0, min(self.bank + spec["delta"], max(0, (len(tracks) - 1) // 20)))
            log(f"Track bank {self.bank + 1}")
        elif not t:
            log("No track selected in SoundCraft.")
        elif target == "mute":
            self.sc.cmd("mix.mute", tracks=[g["id"] for g in group], value=on)
        elif target == "solo":
            self.sc.cmd("mix.solo", tracks=[g["id"] for g in group], value=on)
        elif target == "phase":
            for g in group:
                self.phase[g["id"]] = on
            self.sc.cmd("mix.phase_invert", tracks=[g["id"] for g in group], value=on)
        elif target == "section":
            for g in group:
                ins = self.section_insert(g, spec["section"])
                if not ins:
                    if on:
                        self.ensure_section(g, spec["section"])
                else:
                    self.sc.cmd("mix.insert_active", track=g["id"], slot=ins["slot"], value=on)
        elif target == "param_toggle":
            new = spec["on"] if on else spec["off"]
            for g in group:
                ins = self.section_insert(g, spec["section"])
                if self.ensure_section(g, spec["section"], {spec["param"]: new}) and ins:
                    self.sc.cmd("mix.insert_params", track=g["id"], slot=ins["slot"], params=to_plugin(spec["section"], {spec["param"]: new}))
        self.nudge()
        self.poll(); self.feedback()

    # --- LEDs ---
    def send_led(self, label, value, force=False):
        if SOFTUBE:
            if not self.c1:
                return
            if label in self.c1.ring_pos:
                self.c1.ring(label, value)
            else:
                self.c1.button(label, value)
            self.led[label] = value
            return
        cc = label_to_cc.get(label)
        if not cc:
            return
        if force or self.led.get(cc) != value:
            self.out.send(mido.Message("control_change", channel=cc[0], control=cc[1], value=value))
            self.led[cc] = value

    def feedback(self, force=False):
        with self.fb_lock:              # one LED update at a time, so threads can't fight over the lights
            self._feedback(force)

    def _feedback(self, force=False):
        with self.state_lock:
            t, tracks, now = self.track(), list(self.tracks), time.time()
            touched = {l for l, ts in self.touched.items() if now - ts < TOUCH_HOLD_S}
        for label, spec in LAYOUT["knobs"].items():
            if label in touched:
                continue
            k, dark = 0, not t
            if t:
                target = spec.get("target")
                if target == "volume":
                    k = db_to_knob(t.get("volume_db", 0.0))
                elif target == "pan":
                    pans = t.get("pan") or [0.0]
                    if len(pans) >= 2:                  # stereo: show the width
                        k = int(round(127 * max(0.0, min(1.0, (pans[1] - pans[0]) / 2.0))))
                    else:
                        k = int(round(64 + pans[0] * 63))
                elif target == "trim":
                    k = value_to_knob(spec, t.get("trim_db", self.trim.get(t["id"], 0.0)))
                elif "section" in spec:
                    ins = self.section_insert(t, spec["section"])
                    if ins:
                        params = ins.get("params", {})
                        if "switch" in spec and not params.get(spec["switch"]):
                            k = 0 if spec.get("off_at") == "min" else 127
                        elif spec["param"] in params:
                            k = value_to_knob(spec, params[spec["param"]])
                    elif "switch" in spec:
                        k = 0 if spec.get("off_at") == "min" else 127
            if t and "section" in spec:
                ins = self.section_insert(t, spec["section"])
                if not ins or not ins.get("active", True):
                    k, dark = 0, True             # no plug-in yet, or switched out: its rings go dark
            lspec = SEND_LAYER.get(label) if self.layer else None
            if lspec:
                snd = self.send_of(t, lspec["send"]) if t else None
                k, dark = 0, not snd
                if snd:
                    k = db_to_knob(snd.get("level_db", -144)) if lspec["what"] == "level" else int(round(64 + (snd.get("pan") or 0.0) * 63))
            elif self.layer and spec.get("target") not in ("volume", "pan", "trim"):
                k, dark = 0, True             # in the send layer the other strip rings go dark
            if t and spec.get("target") in ("volume", "pan") and self.automated(t, spec["target"]):
                if int(now * 3) % 2:          # flash: automation is playing this control back
                    k, dark = 0, True
            if SOFTUBE and self.c1:
                self.c1.light_enable(label, not dark)
                if dark:
                    self.led[label] = None    # status window: ring off
                    continue
            self.send_led(label, max(0, min(127, k)), force)
        if SOFTUBE and self.c1:            # extra lights that belong to a section go dark with it
            for section, labels in LAYOUT.get("section_lights", {}).items():
                ins = self.section_insert(t, section) if t else None
                lit = bool(ins) and ins.get("active", True) and not self.layer
                for l in labels:
                    self.c1.light_enable(l, lit)
                bit = {"gate": 64, "comp": 32}.get(section)   # meter switches in position 83 (confirmed on hardware)
                if bit:
                    self.meter_dark[section] = not lit
                    with self.c1.lock:
                        v = self.c1.values[82]
                        nv = (v & ~bit) if not lit else (v | bit)
                        if nv != v:
                            self.c1.values[82], self.c1.dirty = nv, True
                        if nv != v:
                            self.c1.values[82], self.c1.dirty = nv, True
        for label, spec in LAYOUT["buttons"].items():
            on = False
            if t:
                target = spec["target"]
                if target in ("mute", "solo"):
                    on = bool(t.get(target))
                elif target == "phase":
                    on = t.get("phase_invert", self.phase.get(t["id"], False))
                elif target == "section":
                    ins = self.section_insert(t, spec["section"])
                    on = bool(ins and ins.get("active", True))
                elif target == "param_toggle":
                    ins = self.section_insert(t, spec["section"])
                    on = bool(ins and ins.get("params", {}).get(spec["param"]) == spec["on"])
            if self.layer and label in SEND.get("also_flash", []):
                on = int(time.time() * 2.5) % 2 == 0     # flashes in step with the send layer button
            self.send_led(label, 127 if on else 0, force)
        if SEND.get("button") and not SEND.get("shift"):
            blink_on = int(time.time() * 2.5) % 2 == 0      # flashes while the send layer is on
            self.send_led(SEND["button"], 127 if (self.layer and blink_on) else 0, force)
        if LAYOUT.get("group_button"):
            self.send_led(LAYOUT["group_button"], 127 if self.group_mode else 0, force)
        lit = {i for i, tr in enumerate(tracks) if tr["id"] in self.selection or (t and tr["id"] == t["id"])}
        for n, label in enumerate(LAYOUT["track_buttons"]):
            i = self.bank * 20 + n
            if SOFTUBE:   # selected = solid bright; others glow with their signal level
                self.set_track_light(n, label, self.track_led(i, tracks, lit), force)
            else:
                self.send_led(label, 127 if i in lit else 0, force)

    def track_led(self, i, tracks, lit):
        """Track button state: 2 bright, 1 dim, 0 off. Selected = steady bright; others follow their level."""
        if i in lit:
            return 2
        if i >= len(tracks):
            return 0
        if self.meters_unavailable:
            return 1
        db = self.track_db.get(tracks[i]["id"], -144.0)
        bright_db = LAYOUT.get("track_bright_db")      # null = bright is kept for the selected track only
        if bright_db is not None and db >= bright_db:
            return 2
        return 1 if db >= LAYOUT.get("track_dim_db", -50.0) else 0

    def set_track_light(self, n, label, state, force=False):
        if self.c1:
            self.c1.track_light(n + 1, state)
            self.c1.track_pwm(n + 1, state == 1 and LAYOUT.get("track_flash_pwm", False))
        self.led[label] = (0, 63, 127)[state]

    def update_track_leds(self):
        with self.state_lock:
            t, tracks = self.track(), list(self.tracks)
            lit = {i for i, tr in enumerate(tracks) if tr["id"] in self.selection or (t and tr["id"] == t["id"])}
        with self.fb_lock:
            for n, label in enumerate(LAYOUT["track_buttons"]):
                self.set_track_light(n, label, self.track_led(self.bank * 20 + n, tracks, lit))

    # --- meters (needs a SoundCraft with the mix.meters command) ---
    def update_meters(self):
        cfg = LAYOUT.get("meters", {})
        if not cfg or self.meters_unavailable:
            return
        r = self.sc_meters.call("mix.meters")
        if r is None:
            if "unknown method" not in str(self.sc_meters.last_error or ""):
                return                      # a passing hiccup; try again next time
            self.meters_unavailable = True
            log("This SoundCraft has no live meters (mix.meters); the Console 1 meters stay dark. Use our SoundCraft build for meters.")
            return
        now = time.time()
        dt, self.meter_t = now - self.meter_t, now
        with self.state_lock:
            t = self.track()
        row = next((x for x in r.get("tracks", []) if t and x.get("id") == t["id"]), None) if r.get("live") else None
        why = ("live" if row else "not live (SoundCraft isn't playing?)" if not r.get("live")
               else "no row for the selected track" if t else "no track selected")
        if why != getattr(self, "_meter_why", None):        # say once whenever the meter situation changes
            self._meter_why = why
            peak = max((max(x.get("output_db", [-144]) or [-144]) for x in r.get("tracks", [])), default=-144)
            log(f"Meters: {why}; loudest track {peak:.0f} dB" + (f"; {t['name']} in {row.get('input_db')} out {row.get('output_db')}" if row else ""))
        lo, hi = cfg.get("floor_db", -60.0), cfg.get("top_db", 0.0)
        def level(db):
            return max(0.0, min(1.0, (db - lo) / (hi - lo)))
        def gr_level(g):
            return max(0.0, min(1.0, g / cfg.get("gr_full_db", 20.0)))
        want = dict.fromkeys(["input_l", "input_r", "output_l", "output_r", "gate", "comp"], 0.0)
        if row:
            want["input_l"], want["input_r"] = (level(x) for x in row.get("input_db", [-144, -144]))
            want["output_l"], want["output_r"] = (level(x) for x in row.get("output_db", [-144, -144]))
            gr = {i.get("slot"): i.get("gain_reduction_db", 0.0) for i in row.get("inserts", [])}
            for name, section in (("gate", "gate"), ("comp", "comp")):
                ins = self.section_insert(t, section)
                if ins:
                    want[name] = gr_level(gr.get(ins["slot"], 0.0))
        fall = cfg.get("fall_per_s", 1.5) * dt          # meters drop smoothly instead of flickering
        if SOFTUBE:
            rows = r.get("tracks", []) if r.get("live") else []
            if not r.get("live"):
                self.track_db.clear()      # playback stopped: the track lights go out at once instead of fading
            fall_db = LAYOUT.get("track_fall_db_per_s", 30.0) * dt     # drops smoothly, so the lights don't flicker
            seen = set()
            for x in rows:
                v = max(x.get("output_db", [-144.0]) or [-144.0])
                cur = self.track_db.get(x.get("id"), -144.0)
                self.track_db[x.get("id")] = v if v >= cur else max(v, cur - fall_db)
                seen.add(x.get("id"))
            for k in list(self.track_db):
                if k not in seen:
                    self.track_db[k] = max(-144.0, self.track_db[k] - fall_db)
            self.update_track_leds()
        for name, v in want.items():
            cur = self.meter_level.get(name, 0.0)
            self.meter_level[name] = v if v >= cur else max(v, cur - fall)
            if SOFTUBE:
                if self.c1:
                    nm = {"comp": "comp_gr", "gate": "gate_gr"}.get(name, name)
                    if self.meter_dark.get(name):
                        self.meter_level[name] = 0.0
                    self.c1.meter(nm, self.meter_level[name])
                continue
            cc = cfg.get("ccs", {}).get(name)
            val = int(round(self.meter_level[name] * 127))
            if name in cfg.get("reversed", []):
                val = 127 - val                     # this meter counts down on the hardware
            if cc is not None and self.meter_sent.get(cc) != val:
                self.out.send(mido.Message("control_change", channel=0, control=cc, value=val))
                self.meter_sent[cc] = val
        self.meter_row = row

    # --- status window (http://127.0.0.1:7880) ---
    def readable(self, t, label, spec):
        """The value a knob controls, in plain units, for the status window."""
        target = spec.get("target")
        if not t:
            return ""
        if target == "volume":
            db = t.get("volume_db", 0.0)
            return "-inf dB" if db <= -140 else f"{db:+.1f} dB"
        if target == "pan":
            pans = t.get("pan") or [0.0]
            if len(pans) >= 2:
                return f"width {round(100 * (pans[1] - pans[0]) / 2)}%"
            p = pans[0]
            return "C" if abs(p) < 0.01 else f"{'L' if p < 0 else 'R'}{round(abs(p) * 100)}"
        if target == "trim":
            return f"{t.get('trim_db', self.trim.get(t['id'], 0.0)):+.1f} dB"
        if "section" in spec:
            ins = self.section_insert(t, spec["section"])
            if not ins:
                return "—"
            params = ins.get("params", {})
            if "switch" in spec and not params.get(spec["switch"]):
                return "off"
            v, name = params.get(spec["param"]), spec["param"]
            if v is None:
                return ""
            if "freq" in name or name == "tone":
                return f"{v / 1000:.1f} kHz" if v >= 1000 else f"{v:.0f} Hz"
            if name.endswith("_q"):
                return f"Q {v:.2f}"
            if name == "ratio":
                return f"{v:.1f}:1"
            if name == "mix":
                return f"{v:.0f}%"
            if name in ("attack", "release", "hold"):
                return f"{v:.1f} ms" if v < 10 else f"{v:.0f} ms"
            return f"{v:+.1f} dB" if "gain" in name else f"{v:.1f} dB"
        return ""

    def gui_state(self):
        with self.state_lock:
            t, tracks = self.track(), list(self.tracks)
        rings = {label: (self.led.get(label, 0) if SOFTUBE else self.led.get(label_to_cc.get(label), 0)) for label in list(LAYOUT["knobs"])}
        knobs = {}
        for label, spec in LAYOUT["knobs"].items():
            lspec = SEND_LAYER.get(label) if self.layer else None
            if lspec and t:
                snd = self.send_of(t, lspec["send"])
                if not snd:
                    val = "no send" if lspec["what"] == "level" else ""
                elif lspec["what"] == "level":
                    db = snd.get("level_db", -144)
                    val = "-inf dB" if db <= -140 else f"{db:+.1f} dB"
                else:
                    p = snd.get("pan") or 0.0
                    val = "C" if abs(p) < 0.01 else f"{'L' if p < 0 else 'R'}{round(abs(p) * 100)}"
                name = f"Send {chr(65 + lspec['send'])} " + ("level" if lspec["what"] == "level" else "pan")
            else:
                val, name = self.readable(t, label, spec), label
            knobs[label] = {"name": name, "value": val, "ring": rings.get(label, 0)}
        sections = {}
        if t:
            for sec, cfg in SECTIONS.items():
                ins = self.section_insert(t, sec)
                sections[sec] = "missing" if not ins else ("inactive" if not ins.get("active", True) else "on")
        sel = next((i for i, tr in enumerate(tracks) if t and tr["id"] == t["id"]), -1)
        return {
            "soundcraft": self.sc.connected, "console1": bool(self.c1) or not SOFTUBE,
            "fader": self.fader.state() if self.fader else None,
            "track": t["name"] if t else None,
            "stereo": bool(t and len(t.get("pan") or []) >= 2),
            "automated": bool(t and self.automated(t, "volume")),
            "mute": bool(t and t.get("mute")), "solo": bool(t and t.get("solo")),
            "phase": bool(t and t.get("phase_invert", self.phase.get(t["id"]))),
            "layer": self.layer, "bank": self.bank,
            "group": self.group_mode, "group_size": len(self.targets()),
            "tracks": [{"name": tr["name"], "n": i + 1, "selected": i == sel, "grouped": tr["id"] in self.selection, "mute": tr.get("mute"),
                        "solo": tr.get("solo"), "kind": tr.get("kind")} for i, tr in enumerate(tracks)],
            "knobs": knobs, "sections": sections, "log": list(LOG_TAIL),
            "meters": {k: round(v, 3) for k, v in self.meter_level.items()},
            "meters_available": not self.meters_unavailable,
        }

    def serve_gui(self):
        ctl = self

        class H(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_GET(self):
                if self.path.startswith("/state"):
                    body, kind = json.dumps(ctl.gui_state()).encode(), "application/json"
                else:   # read fresh each time, so page edits show on refresh
                    body, kind = (HERE / "status.html").read_bytes(), "text/html; charset=utf-8"
                self.send_response(200)
                self.send_header("Content-Type", kind)
                self.send_header("Cache-Control", "no-store")
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                if self.path == "/quit":
                    self.send_response(200); self.end_headers(); self.wfile.write(b"bye")
                    ctl.quitting = True
                elif self.path.startswith("/select/"):
                    n = int(self.path.rsplit("/", 1)[1])
                    with ctl.state_lock:
                        tracks = list(ctl.tracks)
                    if 0 <= n < len(tracks):
                        ctl.sc.cmd("edit.select", tracks=[tracks[n]["id"]])
                    self.send_response(200); self.end_headers()

        port = LAYOUT.get("gui_port", 7880)
        srv = ThreadingHTTPServer(("127.0.0.1", port), H)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        log(f"Status window: http://127.0.0.1:{port}")

    # --- run ---
    def run(self):
        self.quitting = False
        self.serve_gui()
        self.sc.connect()
        self.poll(); self.feedback(force=True)
        t = self.track()
        log("Ready." + (f" Selected track: {t['name']}" if t else " Select a track to start."))
        if SOFTUBE:
            try:
                self.c1 = Console1Softube(self.on_event)
                self.c1.brightness(LAYOUT.get("brightness", 3))
                self.c1.set_pwm_mode(LAYOUT.get("track_pwm_duty", 0.5), LAYOUT.get("track_pwm_rate_hz", 80))
                log("Console 1 in Softube mode (Shift, holds and per-light brightness available)")
                self.feedback(force=True)
            except RuntimeError as e:
                log(f"{e} - carrying on without it.")
            inp = None
        else:
            inp = mido.open_input(HW["device_in"], callback=self.on_midi)
        if LAYOUT.get("fader", {}).get("enabled", True):
            try:
                from fader_bank import FaderBank
                self.fader = FaderBank(self, log)
                self.fader.feedback()
            except RuntimeError as e:
                log(f"{e} - carrying on without it.")
        self.sc_meters.connect()

        def meter_loop():
            while not self.quitting and not self.meters_unavailable:
                t0 = time.time()
                try:
                    self.update_meters()
                except Exception as e:
                    log("Meter update failed:", e)
                time.sleep(max(0.005, 1 / 30 - (time.time() - t0)))
        threading.Thread(target=meter_loop, daemon=True).start()
        last_poll, last_name = 0, None
        try:
            while not self.quitting:
                self.flush()
                if self.popup and time.time() - self.popup[3] > LAYOUT.get("plugin_popup", {}).get("close_after_s", 2.0):
                    self.hide_plugin()
                if time.time() - last_poll > POLL_S:
                    self.poll(); self.feedback(); last_poll = time.time()
                    t = self.track()
                    if t and t["name"] != last_name:
                        log(f"Track: {t['name']}"); last_name = t["name"]
                        auto = [p for p in ("volume", "pan") if self.automated(t, p)]
                        if auto:
                            log(f"  {t['name']} has {' and '.join(auto)} automation in {t.get('automation_mode')} mode, so its ring flashes;"
                                " set the track's automation to Off to use the knob")
                time.sleep(FLUSH_S)
        except KeyboardInterrupt:
            pass
        finally:
            if self.fader:
                self.fader.close()
            if SOFTUBE:
                if self.c1:
                    self.c1.close()
            else:
                inp.close()
                for cc in list(self.led):
                    self.out.send(mido.Message("control_change", channel=cc[0], control=cc[1], value=0))
            log("Stopped.")


def other_owner():
    """FaderBridge or Softube's display would fight us for the Fader."""
    import subprocess
    def running(pat):
        return subprocess.run(["pgrep", "-f", pat], capture_output=True).returncode == 0
    if running("bridge/main.py") or running("pt_adaptor.py"):
        return "FaderBridge is running. Quit it first (FaderBridge app or panel > Stop), then start this again."
    if running("Console1OSD_APP"):
        return "Softube's Console 1 display (OSD) is open. Quit it first (menu bar icon > Quit), then start this again."
    return None


def _stop_on_signal(*_):
    raise KeyboardInterrupt   # same tidy shutdown as Ctrl-C (LEDs off, "Stopped.")


if __name__ == "__main__":
    import signal
    signal.signal(signal.SIGTERM, _stop_on_signal)
    signal.signal(signal.SIGINT, signal.default_int_handler)   # background starts ignore Ctrl-C otherwise
    _why = other_owner()
    if _why:
        sys.exit(_why)
    _lock = socket.socket()
    try:
        _lock.bind(("127.0.0.1", 47801))   # only one controller at a time
    except OSError:
        sys.exit("Another Console 1 controller is already running. Close its Terminal window first.")
    Controller().run()
