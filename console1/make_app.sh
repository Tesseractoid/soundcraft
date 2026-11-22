#!/bin/bash
# Wraps our SoundCraft build in a proper Mac app ("SoundCraft (Console 1).app") so it gets a menu
# bar, Dock icon and Cmd-Q like the official one. Re-copies the build whenever it's newer.
DIR="$(cd "$(dirname "$0")" && pwd)"
BIN="$DIR/../target/release/soundcraft"
mkdir -p "$HOME/Applications"
APP="$HOME/Applications/SoundCraft (Console 1).app"
[ -x "$BIN" ] || exit 1
if [ ! -f "$APP/Contents/MacOS/soundcraft" ] || ! grep -q autostart "$APP/Contents/MacOS/SoundCraft-console1" 2>/dev/null || [ "$BIN" -nt "$APP/Contents/MacOS/soundcraft" ]; then
  mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
  VER=$(grep -m1 '^version' "$DIR/../Cargo.toml" | cut -d'"' -f2); VER=${VER:-0.0.0}
  sed -e "s/@VERSION@/$VER-console1/; s/@SHORT_VERSION@/${VER%%-*}/g; s/@BUILD_SHA@/console1/" \
      "$DIR/../packaging/macos/Info.plist.in" > "$APP/Contents/Info.plist"
  # The app always opens with the control port, so the Console 1 controller can connect at any time.
  # It also starts console1/autostart.sh, which runs the controller whenever the Console 1 is plugged in.
  cat > "$APP/Contents/MacOS/SoundCraft-console1" <<LAUNCH
#!/bin/bash
HERE="\$(cd "\$(dirname "\$0")" && pwd)"
[ -x "$DIR/autostart.sh" ] && "$DIR/autostart.sh" "\$\$" &
"\$HERE/soundcraft" --control 7801 "\$@"
LAUNCH
  chmod +x "$APP/Contents/MacOS/SoundCraft-console1"
  /usr/bin/plutil -replace CFBundleExecutable -string SoundCraft-console1 "$APP/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleIdentifier -string ai.storyteller.soundcraft.console1 "$APP/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleDisplayName -string "SoundCraft (Console 1)" "$APP/Contents/Info.plist"
  /usr/bin/plutil -replace CFBundleName -string "SoundCraft" "$APP/Contents/Info.plist"
  ICON="$DIR/../assets/app-icon/soundcraft.icns"   # from the repo, so the official app isn't needed
  [ -f "$ICON" ] && cp "$ICON" "$APP/Contents/Resources/SoundCraft.icns"
  cp "$BIN" "$APP/Contents/MacOS/soundcraft"
  codesign --force --deep --sign - "$APP" 2>/dev/null
  touch "$APP"
fi
