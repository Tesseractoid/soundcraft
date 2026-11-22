# Joel's SoundCraft master

This fork (Tesseractoid/soundcraft) holds the master SoundCraft used on both Macs:

* **SoundCraft** from upstream `main`, plus
* **Automation fixes**: faders follow volume automation (also on branch `fix/mixer-automation-fader-display` for the upstream PR)
* **Console 1 live meters**: `mix.meters`, `mix.insert_active`, polarity button
* **`console1/`**: the Console 1 + Fader mk1 controller (Python), its maps and launchers
* The AK 4K plug-in is kept in a separate **private** project, `Tesseractoid/ak-4k-eq`. Clone it next to this folder (GitHub Desktop puts both in Documents/GitHub) and Update SoundCraft Master builds it too.

## Branches

| Branch | What it is |
|---|---|
| `master` | Everything above. Both Macs use this. |
| `main` | Plain upstream SoundCraft. |
| `fix/mixer-automation-fader-display` | Automation fix only, for the upstream pull request. |

## Keeping both Macs in sync

1. In GitHub Desktop, make sure the current branch is **master**.
2. Finished working on a Mac: write a short summary, press **Commit to master**, then **Push origin**.
3. On the other Mac: press **Fetch origin**, then **Pull origin**.
4. Double-click **Update SoundCraft Master** in this folder. It rebuilds SoundCraft, AK 4K (if its private folder is next to this one) and the controller for that Mac.

Each Mac builds its own copy (the laptop is Intel, the Mac Studio is Apple Silicon). Only the code is synced, never the built apps.

## Getting a new SoundCraft release

Upstream changes reach `main` first, then get merged into `master`. Ask Claude to do the merge and check the Console 1 parts still work.
