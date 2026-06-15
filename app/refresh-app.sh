#!/usr/bin/env bash
# Rebuild the PHYSICAL OVERTURE macOS app so it reflects the current source.
#
# Run this after ANY change to:
#   • the frontend  (app/index.html, app/src/*)
#   • the Tauri backend (app/src-tauri/src/*)
#   • the engine crate (engine/src/*)
#
# Why it's needed: Tauri embeds the frontend into the binary at COMPILE time from a
# copied `dist/` folder, so source edits are invisible to the built app until this runs.
#
# Prefers the Tauri CLI (proper .app + .dmg bundle). If the CLI isn't installed it
# falls back to compiling the release binary and swapping it into the existing .app
# bundle in place (ad-hoc re-signed) so Overture.app still launches with the new code.

set -euo pipefail
cd "$(dirname "$0")" # → app/

echo "▸ staging frontend into dist/ …"
rm -rf dist && mkdir dist && cp index.html dist/ && cp -r src dist/

cd src-tauri

if cargo tauri --version >/dev/null 2>&1; then
  echo "▸ cargo tauri build (full bundle) …"
  cargo tauri build
  echo "✅ rebuilt Overture.app + .dmg via cargo tauri build"
  echo "   bundle: $(pwd)/target/release/bundle/macos/Overture.app"
  exit 0
fi

echo "▸ no Tauri CLI — compiling release binary …"
cargo build --release

APP="target/release/bundle/macos/Overture.app"
if [ -d "$APP" ]; then
  # the bundle executable is named after productName ("Overture"); copy over whatever is there
  cp target/release/overture "$APP/Contents/MacOS/$(ls "$APP/Contents/MacOS" | head -1)"
  codesign --force --sign - "$APP" >/dev/null 2>&1 || true
  echo "✅ refreshed $APP (binary swap + ad-hoc sign)"
  echo "   open it:  open \"$(pwd)/$APP\""
else
  echo "⚠️  no .app bundle at $APP yet."
  echo "   Install the Tauri CLI once to create the bundle:"
  echo "     cargo install tauri-cli --version '^2' && cargo tauri build"
  echo "   (the updated raw binary is at $(pwd)/target/release/overture)"
fi
