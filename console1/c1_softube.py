"""Console 1 mk1 in Softube mode: input events and the light block.

Inputs:  SysEx cmd 0x00, data [control id, value]. Buttons: 127 press, 0 release.
         Knobs: one message per click, 127 = clockwise, 0 = anticlockwise.
Lights:  SysEx cmd 0x01, data [counter, 96 values]. Value positions are 1-based (position = index + 1).
         A light shows only if its level is set AND its layer/enable switch is on (see c1_lightmap.json).

The unit must be switched into this mode with cmd 0x20 (value 7F), and kept there by sending
light blocks regularly (this class re-sends at least every 0.5 s).
"""
import json
import threading
import time
from pathlib import Path

import mido

from c1sysex import frame, parse

HERE = Path(__file__).resolve().parent


def _port(names):
    return next((n for n in names if n.lower().startswith("console 1") and "fader" not in n.lower()), None)


class Console1Softube:
    def __init__(self, on_event, keepalive_s=0.5, max_rate_hz=40):
        """on_event(label, value, pressed) is called from the MIDI thread for every control event.
        For buttons `pressed` is True/False; for knobs it is None and `value` is +1 / -1 (one click)."""
        self.inputs = {int(k, 16): v["label"] for k, v in json.loads((HERE / "console1_softube_map.json").read_text()).items()}
        self.lm = json.loads((HERE / "c1_lightmap.json").read_text())
        base = json.loads((HERE / "softube_baseline_frame.json").read_text())[1:]
        self.values = list(base)                       # 96 values, positions 1..96
        for p in self.lm["layers"]["track_layers"] + self.lm["layers"]["enables"]:
            self.values[p - 1] = 127                   # every light allowed; brightness via levels
        self.ring_pos = self.lm["rings"]
        self.button_pos = {**self.lm["buttons"], **{k: v for k, v in self.lm["tracks"].items()}}
        self.three_pos = self.lm["three_state"]
        self.meter_pos = self.lm["meters"]
        self.knob_labels = set(self.ring_pos)          # knobs = controls with a ring
        self.on_event = on_event
        self.keepalive_s, self.min_gap = keepalive_s, 1.0 / max_rate_hz
        self.lock = threading.Lock()
        self.pwm, self.pwm_period, self.pwm_gap, self.pwm_phase = set(), 2, 1.0 / 80, 0   # fast on/off to fake a dimmer light
        self.dirty, self.seq, self.last_sent = True, 0, 0.0
        self.shift = False
        in_name, out_name = _port(mido.get_input_names()), _port(mido.get_output_names())
        if not in_name or not out_name:
            raise RuntimeError("Console 1 not found (is it plugged in, and Softube's display closed?)")
        self.out = mido.open_output(out_name)
        self.inp = mido.open_input(in_name, callback=self._midi)
        self.out.send(self._sysex(0x20, b"\x7f"))      # switch to Softube mode
        self._running = True
        threading.Thread(target=self._sender, daemon=True).start()

    # ---------- input ----------
    def _midi(self, msg):
        if msg.type != "sysex":
            return
        r = parse(bytes([0xF0] + list(msg.data) + [0xF7]))
        if not r or r[0] != 0x00 or len(r[1]) < 2:
            return
        cid, val = r[1][0], r[1][1]
        label = self.inputs.get(cid)
        if not label:
            return
        if label == "Shift":
            self.shift = val > 0
        if label in self.knob_labels:
            self.on_event(label, 1 if val >= 64 else -1, None)
        else:
            self.on_event(label, val, val > 0)

    # ---------- lights ----------
    def _set(self, pos, value):
        value = max(0, min(127, int(value)))
        with self.lock:
            if self.values[pos - 1] != value:
                self.values[pos - 1] = value
                self.dirty = True

    def ring(self, label, value):
        """Knob ring dot position, 0..127."""
        if label in self.ring_pos:
            self._set(self.ring_pos[label], value)

    def button(self, label, level):
        """Button light: 0 off, 63 dim, 127 bright (any value in between works)."""
        if label in self.button_pos:
            self._set(self.button_pos[label], level)

    # Track buttons 1-20 have a separate "bright" switch, packed 7 per byte, highest bit first:
    # 84 bit1 = track 1, bit0 = track 2; 85 = tracks 3-9; 86 = tracks 10-16; 87 bits 6..3 = tracks 17-20.
    # (Confirmed on hardware: 85 bit0 = track 9.) Level 0 = off, level >0 = dim, plus switch = bright.
    BRIGHT_BITS = [(84, 1), (84, 0)] + [(85, 6 - k) for k in range(7)] + [(86, 6 - k) for k in range(7)] + [(87, 6 - k) for k in range(4)]

    def track_light(self, n, state):
        """n: track button 1..20; state: 0 off, 1 dim, 2 bright."""
        label = str(n)
        if label not in self.button_pos or not 1 <= n <= 20:
            return
        pos, bit = self.BRIGHT_BITS[n - 1]
        with self.lock:
            v = self.values[pos - 1]
            nv = (v | (1 << bit)) if state == 2 else (v & ~(1 << bit))
            if nv != v:
                self.values[pos - 1] = nv
                self.dirty = True
        self._set(self.button_pos[label], 127 if state else 0)

    def set_pwm_mode(self, duty=0.5, rate_hz=80):
        """Lights in pwm mode are on 1 frame in every round(1/duty), sent at rate_hz while any are active."""
        self.pwm_period = max(1, round(1.0 / max(0.05, min(1.0, duty))))
        self.pwm_gap = 1.0 / max(1, rate_hz)

    def track_pwm(self, n, on):
        label = str(n)
        if label in self.button_pos:
            with self.lock:
                (self.pwm.add if on else self.pwm.discard)(self.button_pos[label])

    # Enable switches 76-82 (confirmed on hardware 2026-11-18): light position P (22..70) is
    # switched by position 76 + (P - 22) // 7, bit 6 - (P - 22) % 7. Off = that light is fully dark.
    def light_enable(self, label, on):
        pos = self.ring_pos.get(label) or self.button_pos.get(label) or self.three_pos.get(label)
        if not pos or not 22 <= pos <= 70:
            return
        epos, bit = 76 + (pos - 22) // 7, 6 - (pos - 22) % 7
        with self.lock:
            v = self.values[epos - 1]
            nv = (v | (1 << bit)) if on else (v & ~(1 << bit))
            if nv != v:
                self.values[epos - 1] = nv
                self.dirty = True

    def three_state(self, label, index):
        """3-way indicators (EQ types, Order, External Sidechain): index 0, 1 or 2."""
        if label in self.three_pos:
            self._set(self.three_pos[label], (0, 63, 127)[max(0, min(2, index))])

    def meter(self, name, level):
        """name: input_l, input_r, output_l, output_r, comp_gr. level 0..1."""
        if name in self.meter_pos:
            self._set(self.meter_pos[name], round(max(0.0, min(1.0, level)) * 127))

    def brightness(self, level):
        """Whole panel: 0-2 off, 3 normal (Softube), up to 15 very bright."""
        self._set(self.lm["brightness_global"], level)

    def section_enables(self, positions_values):
        """Advanced: write enable bitfields directly, e.g. to darken a section's rings."""
        for p, v in positions_values.items():
            self._set(int(p), v)

    def _sysex(self, cmd, data):
        return mido.Message("sysex", data=list(frame(cmd, bytes(data))[1:-1]))

    def _sender(self):
        while self._running:
            now = time.time()
            with self.lock:
                if self.pwm:
                    due = now - self.last_sent >= self.pwm_gap
                else:
                    due = (self.dirty and now - self.last_sent >= self.min_gap) or now - self.last_sent >= self.keepalive_s
                if due:
                    self.seq = (self.seq + 1) % 128
                    vals = self.values
                    if self.pwm:
                        self.pwm_phase = (self.pwm_phase + 1) % self.pwm_period
                        if self.pwm_phase:                 # off part of the cycle
                            vals = list(vals)
                            for p in self.pwm:
                                vals[p - 1] = 0
                    data = [self.seq] + vals
                    self.dirty = False
                    self.last_sent = now
            if due:
                try:
                    self.out.send(self._sysex(0x01, data))
                except OSError:
                    pass
            time.sleep(0.002)

    def close(self, dark=True):
        self._running = False
        time.sleep(0.02)
        if dark:
            self.out.send(self._sysex(0x01, [0] + [0] * 96))
        self.inp.close()
        self.out.close()
