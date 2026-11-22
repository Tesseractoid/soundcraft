"""Softube Console 1 Fader mk1, driven directly (no Softube software). Findings: fader_findings.txt.

Reading: direct mode (SysEx cmd 0x20 7F, repeated every 0.5 s) -> faders in 1024 steps, touch, every button.
Motors:  plain CC 100 + 2*(n-1), 7-bit; the Fader accepts these while in direct mode.
"""
import threading
import time

import mido

from c1sysex import frame, parse

N = 10
TOUCH = {20 - 2 * k: k for k in range(N)}            # cmd 0x00 ids
SELECT = {0x7A - 6 * k: k for k in range(N)}
MUTE = {0x7A - 6 * k + 2: k for k in range(N)}
SOLO = {0x7A - 6 * k + 4: k for k in range(N)}
BUTTONS = {0x1C: "layer", 0x1A: "assign", 0x42: "page_left", 0x40: "page_right", 0x16: "shift",
           0x36: "send1", 0x34: "send2", 0x32: "send3", 0x2E: "play", 0x30: "rewind", 0x2C: "forward"}
KNOB_ID = 0x14

# Lights: plain CC 127/0, accepted in direct mode too (probe 8). 5 CCs per strip, fader 1 highest.
def LED_SELECT(k): return 81 - 5 * k
def LED_MUTE(k): return 82 - 5 * k
def LED_SOLO(k): return 84 - 5 * k
LED_LAYER, LED_PAGE_LEFT, LED_PAGE_RIGHT = 14, 34, 33
PAN_DOTS = None                    # CC 96 only reaches the first 3 dots; ring not mapped yet (off for now)

# Softube's own fader scale (from the OSD; FaderBridge measured it against Pro Tools within ~0.2 dB):
# physical travel 0..0.7 -> 0..0.75 and 0.7..1 -> 0.75..1 on this 21-point table (top to bottom, 5% steps).
_TABLE = [12, 9.6, 7.2, 4.8, 2.4, 0, -2.6, -5.3, -7.0, -10.6, -13.1, -15.8, -18.5,
          -21.7, -25.9, -30, -34.2, -38.4, -49.7, -66.5, -144]


def _phys_to_h(p):
    return p * 0.75 / 0.7 if p <= 0.7 else 0.75 + (p - 0.7) * 0.25 / 0.3


def _h_to_phys(h):
    return h * 0.7 / 0.75 if h <= 0.75 else 0.7 + (h - 0.75) * 0.3 / 0.25


def pos_to_db(v):
    """Fader position 0..1023 -> dB."""
    if v <= 0:
        return -144.0
    i = (1.0 - _phys_to_h(min(1023, v) / 1023.0)) / 0.05
    a = min(int(i), 19)
    f = i - a
    return _TABLE[a] + (_TABLE[a + 1] - _TABLE[a]) * f


def db_to_pos(db):
    """dB -> fader position 0..1023."""
    if db >= _TABLE[0]:
        return 1023
    if db <= -143:
        return 0
    for a in range(20):
        hi, lo = _TABLE[a], _TABLE[a + 1]
        if lo <= db <= hi:
            f = (hi - db) / (hi - lo) if hi != lo else 0.0
            h = 1.0 - (a + f) * 0.05
            return int(round(_h_to_phys(h) * 1023))
    return 0


def _port(names):
    return next((n for n in names if "fader" in n.lower()), None)


class FaderMk1:
    def __init__(self, on_event):
        """on_event(kind, index, value) from the MIDI thread:
        ('move', k, pos 0..1023)  ('touch', k, True/False)  ('select'|'mute'|'solo', k, pressed)
        ('button', name, pressed)  ('knob', None, +1/-1)."""
        self.on_event = on_event
        in_name, out_name = _port(mido.get_input_names()), _port(mido.get_output_names())
        if not in_name or not out_name:
            raise RuntimeError("Fader not found (is it plugged in, and Softube's display closed?)")
        self.out = mido.open_output(out_name)
        self.inp = mido.open_input(in_name, callback=self._midi)
        self.lock = threading.Lock()
        self.touched = [False] * N
        self.pos = [None] * N              # last position reported by the hardware
        self.motor_sent = [None] * N       # last CC value sent to each motor
        self.held = set()                  # button names currently held
        self.lit = {}                      # CC -> value last sent to a light
        self._running = True
        self._sx(0x20, [0x7F])
        threading.Thread(target=self._keepalive, daemon=True).start()

    def _sx(self, cmd, data):
        with self.lock:
            self.out.send(mido.Message.from_bytes(list(frame(cmd, bytes(b & 0x7F for b in data)))))

    def _keepalive(self):
        while self._running:
            try:
                self._sx(0x20, [0x7F])
            except Exception:
                pass
            time.sleep(0.5)

    def _midi(self, msg):
        if msg.type != "sysex":
            return
        r = parse(bytes(msg.bytes()))
        if not r:
            return
        cmd, d = r[0], list(r[1])
        try:
            if cmd == 0x18 and len(d) >= 3:
                if d[0] == KNOB_ID:
                    self.on_event("knob", None, 1 if d[1] >= 64 else -1)
                elif d[0] % 2 == 0 and d[0] // 2 < N:
                    k = d[0] // 2
                    v = ((d[2] & 0x3F) << 7) | d[1]
                    self.pos[k] = v
                    if d[2] & 0x40:          # only hand moves (the touch flag is set while held)
                        self.on_event("move", k, v)
            elif cmd == 0x00 and len(d) >= 2:
                cid, down = d[0], d[1] >= 64
                if cid in TOUCH:
                    k = TOUCH[cid]
                    self.touched[k] = down
                    self.on_event("touch", k, down)
                elif cid in SELECT:
                    self.on_event("select", SELECT[cid], down)
                elif cid in MUTE:
                    self.on_event("mute", MUTE[cid], down)
                elif cid in SOLO:
                    self.on_event("solo", SOLO[cid], down)
                elif cid in BUTTONS:
                    name = BUTTONS[cid]
                    (self.held.add if down else self.held.discard)(name)
                    self.on_event("button", name, down)
        except Exception as e:                # never let one bad message kill the MIDI thread
            print("fader event error:", e, flush=True)

    @property
    def shift(self):
        return "shift" in self.held

    def motor(self, k, pos, force=False):
        """Move fader k (0-9) to pos 0..1023, unless a hand is on it."""
        if self.touched[k]:
            return
        v = max(0, min(127, int(round(pos * 127 / 1023))))
        if force or self.motor_sent[k] != v:
            with self.lock:
                self.out.send(mido.Message("control_change", channel=0, control=100 + 2 * k, value=v))
            self.motor_sent[k] = v

    def light(self, n, value, force=False):
        """Set a light (CC n) to 127 (on) / 0 (off), or any value for the pan dots; only sends changes."""
        value = 127 if value is True else 0 if value is False else max(0, min(127, int(value)))
        if force or self.lit.get(n) != value:
            with self.lock:
                self.out.send(mido.Message("control_change", channel=0, control=n, value=value))
            self.lit[n] = value

    def lights_off(self):
        for k in range(N):
            for n in (LED_SELECT(k), LED_MUTE(k), LED_SOLO(k)):
                self.light(n, False, force=True)
        for n in (LED_LAYER, LED_PAGE_LEFT, LED_PAGE_RIGHT):
            self.light(n, False, force=True)

    def close(self):
        self._running = False
        try:
            self.lights_off()
            self.inp.close(); self.out.close()
        except Exception:
            pass
