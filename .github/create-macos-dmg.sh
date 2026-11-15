#!/usr/bin/env bash
# Styled macOS DMG: Tarn.app + Applications, brand background, fixed icon layout.
# Usage: create-macos-dmg.sh <Tarn.app> <output.dmg> [volume-name]
set -euo pipefail

APP_PATH="${1:?Usage: $0 <App.app> <out.dmg> [volname]}"
DMG_PATH="${2:?Usage: $0 <App.app> <out.dmg> [volname]}"
VOL_NAME="${3:-Tarn}"

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BG_SRC="$ROOT/packaging/macos/dmg-background.png"
VOL_ICON_SRC=""
for cand in \
  "$ROOT/src-tauri/icons/icon.icns" \
  "$ROOT/src-tauri/icons/icon.png"; do
  if [[ -f "$cand" ]]; then
    VOL_ICON_SRC="$cand"
    break
  fi
done

if [[ ! -d "$APP_PATH" ]]; then
  echo "App bundle not found: $APP_PATH" >&2
  exit 1
fi
if [[ ! -f "$BG_SRC" ]]; then
  echo "Missing DMG background: $BG_SRC" >&2
  echo "Run: node scripts/make-dmg-background.mjs" >&2
  exit 1
fi

APP_NAME="$(basename "$APP_PATH")"
STAGE="$(mktemp -d "${TMPDIR:-/tmp}/tarn-dmg.XXXXXX")"
RW_DMG="$STAGE/rw.dmg"
MOUNT=""
cleanup() {
  if [[ -n "$MOUNT" && -d "$MOUNT" ]]; then
    hdiutil detach "$MOUNT" -quiet -force 2>/dev/null || true
  fi
  # Detach by volume name if mount path was lost.
  if [[ -d "/Volumes/${VOL_NAME}" ]]; then
    hdiutil detach "/Volumes/${VOL_NAME}" -quiet -force 2>/dev/null || true
  fi
  rm -rf "$STAGE"
}
trap cleanup EXIT

mkdir -p "$(dirname "$DMG_PATH")"
rm -f "$DMG_PATH" "${DMG_PATH}.tmp.dmg"

# --- stage contents ---
mkdir -p "$STAGE/content/.background"
cp -R "$APP_PATH" "$STAGE/content/$APP_NAME"
ln -s /Applications "$STAGE/content/Applications"
cp "$BG_SRC" "$STAGE/content/.background/background.png"

# Optional volume icon (.VolumeIcon.icns).
if [[ -n "$VOL_ICON_SRC" ]]; then
  if [[ "$VOL_ICON_SRC" == *.icns ]]; then
    cp "$VOL_ICON_SRC" "$STAGE/content/.VolumeIcon.icns"
  elif command -v sips >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
    ICONSET="$STAGE/vol.iconset"
    mkdir -p "$ICONSET"
    sips -z 16 16     "$VOL_ICON_SRC" --out "$ICONSET/icon_16x16.png" >/dev/null
    sips -z 32 32     "$VOL_ICON_SRC" --out "$ICONSET/diana.bogdan@example.org" >/dev/null
    sips -z 32 32     "$VOL_ICON_SRC" --out "$ICONSET/icon_32x32.png" >/dev/null
    sips -z 64 64     "$VOL_ICON_SRC" --out "$ICONSET/ivan.p@example.net" >/dev/null
    sips -z 128 128   "$VOL_ICON_SRC" --out "$ICONSET/icon_128x128.png" >/dev/null
    sips -z 256 256   "$VOL_ICON_SRC" --out "$ICONSET/wendy.h@example.net" >/dev/null
    sips -z 256 256   "$VOL_ICON_SRC" --out "$ICONSET/icon_256x256.png" >/dev/null
    sips -z 512 512   "$VOL_ICON_SRC" --out "$ICONSET/wendy.h@example.net" >/dev/null
    sips -z 512 512   "$VOL_ICON_SRC" --out "$ICONSET/icon_512x512.png" >/dev/null
    sips -z 1024 1024 "$VOL_ICON_SRC" --out "$ICONSET/walt.e@example.net" >/dev/null
    iconutil -c icns "$ICONSET" -o "$STAGE/content/.VolumeIcon.icns"
  fi
fi

# Detach leftovers (flaky CI runners).
if [[ -d "/Volumes/${VOL_NAME}" ]]; then
  hdiutil detach "/Volumes/${VOL_NAME}" -quiet -force 2>/dev/null || true
  sleep 1
fi

# Size: contents + headroom for Finder metadata / DS_Store.
SIZE_MB="$(du -sm "$STAGE/content" | awk '{print $1}')"
SIZE_MB=$(( SIZE_MB + 40 ))
if (( SIZE_MB < 80 )); then SIZE_MB=80; fi

attempt=1
until hdiutil create \
  -volname "$VOL_NAME" \
  -srcfolder "$STAGE/content" \
  -ov \
  -fs HFS+ \
  -format UDRW \
  -size "${SIZE_MB}m" \
  "$RW_DMG"; do
  if (( attempt >= 6 )); then
    echo "hdiutil create (UDRW) failed after ${attempt} attempts" >&2
    exit 1
  fi
  echo "hdiutil create busy; retry ${attempt}/6…" >&2
  sleep $(( attempt * 2 ))
  attempt=$(( attempt + 1 ))
done

ATTACH_OUT="$(hdiutil attach -readwrite -noverify -noautoopen "$RW_DMG")"
MOUNT="$(echo "$ATTACH_OUT" | awk '/\/Volumes\//{print $NF; exit}')"
if [[ -z "$MOUNT" || ! -d "$MOUNT" ]]; then
  echo "Failed to mount RW DMG" >&2
  echo "$ATTACH_OUT" >&2
  exit 1
fi

# Custom volume icon bit (hasCustomIcon).
if [[ -f "$MOUNT/.VolumeIcon.icns" ]] && command -v SetFile >/dev/null 2>&1; then
  SetFile -a C "$MOUNT" || true
fi

# Hide helper files from the icon view.
if command -v SetFile >/dev/null 2>&1; then
  SetFile -a V "$MOUNT/.background" 2>/dev/null || true
  [[ -f "$MOUNT/.VolumeIcon.icns" ]] && SetFile -a V "$MOUNT/.VolumeIcon.icns" 2>/dev/null || true
fi

# Finder window chrome + icon positions (logical coords for a 660×400 window).
# Background PNG is @2x (1320×800); Finder scales it.
osascript <<EOF
tell application "Finder"
  tell disk "$VOL_NAME"
    open
    set current view of container window to icon view
    set toolbar visible of container window to false
    set statusbar visible of container window to false
    set bounds of container window to {280, 160, 940, 560}
    set theViewOptions to the icon view options of container window
    set arrangement of theViewOptions to not arranged
    set icon size of theViewOptions to 128
    set text size of theViewOptions to 12
    set background picture of theViewOptions to file ".background:background.png"
    delay 0.4
    set position of item "$APP_NAME" of container window to {180, 210}
    set position of item "Applications" of container window to {480, 210}
    update without registering applications
    delay 1
    close
    open
    delay 0.6
    close
  end tell
end tell
EOF

sync
sleep 1

# Eject cleanly before compress.
hdiutil detach "$MOUNT" -quiet || hdiutil detach "$MOUNT" -force
MOUNT=""

# Compress to final UDZO.
attempt=1
until hdiutil convert "$RW_DMG" \
  -format UDZO \
  -imagekey zlib-level=9 \
  -o "$DMG_PATH"; do
  if (( attempt >= 6 )); then
    echo "hdiutil convert failed after ${attempt} attempts" >&2
    exit 1
  fi
  echo "hdiutil convert busy; retry ${attempt}/6…" >&2
  sleep $(( attempt * 2 ))
  attempt=$(( attempt + 1 ))
done

# Internet-enable so Safari opens the window on download (best-effort).
if hdiutil internet-enable -help >/dev/null 2>&1; then
  hdiutil internet-enable -yes "$DMG_PATH" 2>/dev/null || true
fi

echo "Created $DMG_PATH"
