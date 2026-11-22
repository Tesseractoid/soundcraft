"""The Fader mk1 inside the SoundCraft controller: 10 faders on SoundCraft's track faders.

Layout (Joel's):
  - All 10 faders are a bank of tracks. Page left/right moves the bank.
  - Focus fader: Shift + Select on any fader makes that track the FOCUS track; it then lives on fader 10,
    faders 1-9 carry the bank, and the bank pages by 9. Shift + Select on fader 10 clears focus.
    Plain selecting never moves a track onto the focus fader, so the focus track isn't lost.
  - Layer: HOLD Layer and press Select on tracks to put them in (or take them out of) the layer.
    A short press of Layer flips the faders between the full bank and the layer.
    The button next to Layer (Assign) adds/removes the Console 1's selected track to/from the layer.
  - Select on a fader selects that track (fader 10's Select too); with the Console 1's Group on, it adds/removes.
  - Mute / Solo per fader. Touch + Shift = 0 dB. The Fader's knob is the Pan knob of the selected track.
"""
import threading
import time

import fader_mk1 as hwmap
from fader_mk1 import N, FaderMk1, db_to_pos, pos_to_db

RELEASE_HOLD_S = 0.6        # after letting go, wait this long before the motor follows SoundCraft again
RATE_HZ = 25                # how often the faders read SoundCraft (on their own connection)


class FaderBank:
    def __init__(self, ctl, log):
        self.ctl, self.log = ctl, log
        self.hw = FaderMk1(self.on_event)
        self.view = "bank"           # "bank" or "layer"
        self.page = 0
        self.focus_id = None
        self.layer_ids = []
        self.layer_used = False      # did Select get pressed while Layer was held?
        self.grab = {}               # fader -> track id it was touched on
        self.release_t = [0.0] * N
        self.pending = {}            # fader -> latest hand position, sent by flush()
        self.moved_db = {}           # track id -> (dB we set, time), so the motor doesn't fight a stale poll
        self.levels = {}             # track id -> track (summary), refreshed RATE_HZ times a second
        self.sc = type(ctl.sc)(ctl.sc.port)      # our own line to SoundCraft: never waits behind the knobs
        self.sc.connect()
        self.running = True
        threading.Thread(target=self.loop, daemon=True).start()
        self.log("Fader mk1 connected (1024-step faders, motors on)")

    def loop(self):
        while self.running:
            t0 = time.time()
            try:
                self.flush()
                r = self.sc.call("session.inspect", {"detail": "summary"})
                if r:
                    self.levels = {t["id"]: t for t in r.get("tracks", [])}
                    if self.share(self.levels):
                        self.ctl.feedback()          # Console 1 lights follow at the faders' speed
                self.feedback()
            except Exception as e:
                self.log("Fader update failed:", e)
            time.sleep(max(0.005, 1 / RATE_HZ - (time.time() - t0)))

    # ---------- which track is on which fader ----------
    def source(self, tracks):
        if self.view == "layer":
            return [t for t in tracks if t["id"] in self.layer_ids]
        return tracks

    def per_page(self, tracks):
        return N - 1 if self.focus_track(tracks) else N

    def focus_track(self, tracks):
        return next((t for t in tracks if t["id"] == self.focus_id), None) if self.focus_id else None

    def slots(self, tracks=None):
        if tracks is None:
            with self.ctl.state_lock:
                tracks = list(self.ctl.tracks)
        src, f = self.source(tracks), self.focus_track(tracks)
        per = N - 1 if f else N
        pages = max(1, -(-len(src) // per))
        self.page = max(0, min(self.page, pages - 1))
        out = src[self.page * per: self.page * per + per]
        out += [None] * (per - len(out))
        if f:
            out.append(f)
        return out

    def track_on(self, k):
        s = self.slots()
        return s[k] if k < len(s) else None

    # ---------- hardware events (MIDI thread) ----------
    def on_event(self, kind, k, value):
        if kind == "move":
            self.pending[k] = value
            return
        if kind == "touch":
            if value:
                t = self.track_on(k)
                self.grab[k] = t["id"] if t else None
                if t and self.hw.shift:                       # Shift + touch = 0 dB
                    self.sc.cmd("mix.volume", tracks=[t["id"]], db=0.0)
                    self.moved_db[t["id"]] = (0.0, time.time())
                    self.log(f"Fader {k + 1}: {t['name']} to 0 dB")
            else:
                self.release_t[k] = time.time()
            return
        if kind == "knob":
            if "Pan" in self.ctl.LAYOUT_KNOBS:
                self.ctl.on_event("Pan", value, None)
            return
        if not value:
            if kind == "button" and k == "layer":
                if not self.layer_used:
                    self.flip_layer()
            return                                            # everything else acts on press
        if kind == "button":
            self.button(k)
        elif kind in ("select", "mute", "solo"):
            t = self.track_on(k)
            if not t:
                return
            getattr(self, kind)(k, t)

    def button(self, name):
        with self.ctl.state_lock:
            tracks = list(self.ctl.tracks)
        if name == "layer":
            self.layer_used = False
        elif name in ("page_left", "page_right"):
            per = self.per_page(tracks)
            pages = max(1, -(-len(self.source(tracks)) // per))
            new = max(0, min(pages - 1, self.page + (1 if name == "page_right" else -1)))
            if new != self.page:
                self.page = new
                self.log(f"Faders: {'layer' if self.view == 'layer' else 'bank'} page {new + 1} of {pages}")
        elif name == "assign":
            t = self.ctl.track()
            if t:
                self.toggle_layer(t)

    def select(self, k, t):
        if "layer" in self.hw.held:                           # hold Layer + Select = in/out of the layer
            self.layer_used = True
            self.toggle_layer(t)
            return
        if self.hw.shift:                                     # Shift + Select = focus
            if self.focus_id == t["id"]:
                self.focus_id = None
                self.log(f"Focus off ({t['name']} back in the bank)")
            else:
                self.focus_id = t["id"]
                self.log(f"Focus fader: {t['name']} (faders 1-9 page by 9)")
            return
        ctl = self.ctl
        with ctl.state_lock:
            sel = list(ctl.selection)
        if ctl.group_mode:
            if t["id"] in sel and len(sel) > 1:
                sel.remove(t["id"])
            elif t["id"] not in sel:
                sel.append(t["id"])
            if ctl.selected not in sel:
                ctl.selected = sel[0]
            ctl.sc.cmd("edit.select", tracks=sel)
        else:
            ctl.selected = t["id"]
            ctl.sc.cmd("edit.select", tracks=[t["id"]])
            self.log(f"Selected {t['name']} (fader {k + 1})")
        ctl.poll(); ctl.feedback(force=True)

    def mute(self, k, t):
        t = self.levels.get(t["id"], t)
        self.sc.cmd("mix.mute", tracks=[t["id"]], value=not t.get("mute"))

    def solo(self, k, t):
        t = self.levels.get(t["id"], t)
        self.sc.cmd("mix.solo", tracks=[t["id"]], value=not t.get("solo"))

    def toggle_layer(self, t):
        if t["id"] in self.layer_ids:
            self.layer_ids.remove(t["id"])
            self.log(f"Layer: took out {t['name']} ({len(self.layer_ids)} in the layer)")
        else:
            self.layer_ids.append(t["id"])
            self.log(f"Layer: added {t['name']} ({len(self.layer_ids)} in the layer)")
        if self.view == "layer" and not self.layer_ids:
            self.view = "bank"

    def flip_layer(self):
        if self.view == "bank" and not self.layer_ids:
            self.log("The layer is empty: hold Layer and press Select on the tracks you want in it")
            return
        self.view = "layer" if self.view == "bank" else "bank"
        self.page = 0
        self.log("Faders: LAYER" if self.view == "layer" else "Faders: full bank")

    # ---------- main loop ----------
    def flush(self):
        """Send hand moves to SoundCraft."""
        if not self.pending:
            return
        todo, self.pending = self.pending, {}
        for k, pos in todo.items():
            tid = self.grab.get(k)
            if tid is None:
                t = self.track_on(k)
                tid = t["id"] if t else None
            if tid is None:
                continue
            db = round(pos_to_db(pos), 2)
            self.sc.cmd("mix.volume", tracks=[tid], db=db)
            self.moved_db[tid] = (db, time.time())
            if self.share({tid: {"volume_db": db}}):
                self.ctl.feedback()                  # the Console 1's volume ring moves with your hand

    SHARED = ("volume_db", "mute", "solo", "pan")

    def share(self, fresh):
        """Copy the fresh levels into the controller's tracks; True if the Console 1's track changed."""
        ctl, hit = self.ctl, False
        with ctl.state_lock:
            for t in ctl.tracks:
                f = fresh.get(t["id"])
                if not f:
                    continue
                for key in self.SHARED:
                    if key in f and t.get(key) != f[key]:
                        t[key] = f[key]
                        hit = hit or t["id"] == ctl.selected
        return hit

    def lights(self, slots, tracks):
        """Select / Mute / Solo, Layer, Page and the pan dots follow SoundCraft."""
        hw, ctl = self.hw, self.ctl
        with ctl.state_lock:
            sel = set(ctl.selection) | ({ctl.selected} if ctl.selected else set())
            shown = ctl.track()
        for k, t in enumerate(slots):
            lv = self.levels.get(t["id"], t) if t else {}
            hw.light(hwmap.LED_SELECT(k), bool(t) and t["id"] in sel)
            hw.light(hwmap.LED_MUTE(k), bool(lv.get("mute")))
            hw.light(hwmap.LED_SOLO(k), bool(lv.get("solo")))
        hw.light(hwmap.LED_LAYER, self.view == "layer")
        per = self.per_page(tracks)
        pages = max(1, -(-len(self.source(tracks)) // per))
        hw.light(hwmap.LED_PAGE_LEFT, self.page > 0)
        hw.light(hwmap.LED_PAGE_RIGHT, self.page < pages - 1)
        if shown and hwmap.PAN_DOTS is not None:             # pan dots: the knob's track (the selected one)
            pans = (self.levels.get(shown["id"], shown).get("pan")) or [0.0]
            pos = 127 * max(0.0, min(1.0, (pans[1] - pans[0]) / 2.0)) if len(pans) >= 2 else 64 + pans[0] * 63
            hw.light(hwmap.PAN_DOTS, round(pos))

    def feedback(self):
        """Motors and lights follow SoundCraft."""
        now = time.time()
        with self.ctl.state_lock:
            tracks = list(self.ctl.tracks)
        slots = self.slots(tracks)
        try:
            self.lights(slots, tracks)
        except Exception as e:
            self.log("Fader lights failed:", e)
        for k, t in enumerate(slots):
            if self.hw.touched[k] or now - self.release_t[k] < RELEASE_HOLD_S or k in self.pending:
                continue
            if not t:
                self.hw.motor(k, 0)
                continue
            db = self.levels.get(t["id"], t).get("volume_db", 0.0)
            mine = self.moved_db.get(t["id"])
            if mine and now - mine[1] < 1.0:                  # we just set it; the poll may be older
                db = mine[0]
            self.hw.motor(k, db_to_pos(db))

    def state(self):
        s = self.slots()
        return {"view": self.view, "page": self.page, "layer_size": len(self.layer_ids),
                "focus": next((t["name"] for t in s[-1:] if t and t["id"] == self.focus_id), None),
                "slots": [None if not t else {
                    "name": t["name"], "touched": self.hw.touched[k],
                    "db": self.levels.get(t["id"], t).get("volume_db", 0.0),
                    "mute": self.levels.get(t["id"], t).get("mute"), "solo": self.levels.get(t["id"], t).get("solo"),
                    "focus": t["id"] == self.focus_id, "layer": t["id"] in self.layer_ids}
                    for k, t in enumerate(s)]}

    def close(self):
        self.running = False
        self.hw.close()
