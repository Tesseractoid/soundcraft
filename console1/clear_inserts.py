#!/usr/bin/env python3
"""Remove every insert plug-in from every track in the open SoundCraft session (undoable in SoundCraft)."""
import json, socket
s = socket.create_connection(("127.0.0.1", 7801), timeout=30); f = s.makefile("rw"); n = [0]
def call(m, p=None):
    n[0] += 1; f.write(json.dumps({"id": n[0], "method": m, "params": p or {}}) + "\n"); f.flush(); return json.loads(f.readline())
sess = call("session.inspect", {"detail": "full"})["result"]
removed = 0
for t in sess["tracks"]:
    for ins in sorted([i for i in t.get("inserts") or [] if i], key=lambda i: -i["slot"]):
        r = call("engine.execute", {"command": "mix.insert_remove", "params": {"track": t["id"], "slot": ins["slot"]}})
        ok = r.get("ok"); removed += bool(ok)
        print(f"{t['name']}: {ins['plugin']} in insert {chr(97 + ins['slot']).upper()} -> {'removed' if ok else r.get('error')}")
print(f"\nRemoved {removed} plug-ins. (Edit > Undo in SoundCraft brings them back.)")
