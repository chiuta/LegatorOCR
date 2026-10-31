#!/usr/bin/env bash
# Build and package LegatorOCR (CLI + GUI) for one target triple.
#   scripts/package.sh <target-triple> [version]
# Output: dist/LegatorOCR-<version>-<target>.(tar.gz|zip) and a .sha256 next to it.
set -euo pipefail

TARGET="${1:?usage: package.sh <target-triple> [version]}"
cd "$(dirname "$0")/.."
VERSION="${2:-$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)}"
NAME="LegatorOCR-${VERSION}-${TARGET}"
OUT="dist/${NAME}"

cargo build --release --locked --target "$TARGET" -p legatorocr-cli -p legatorocr-gui
BIN="target/${TARGET}/release"

rm -rf "$OUT" && mkdir -p "$OUT"
EXE=""; case "$TARGET" in *windows*) EXE=".exe";; esac
cp "$BIN/legatorocr${EXE}" "$OUT/"
cp README.md LICENSE "$OUT/"

case "$TARGET" in
  *apple*)
    APP="$OUT/LegatorOCR.app/Contents"
    mkdir -p "$APP/MacOS"
    cp "$BIN/legatorocr-gui" "$APP/MacOS/"
    cat > "$APP/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>LegatorOCR</string>
  <key>CFBundleDisplayName</key><string>LegatorOCR</string>
  <key>CFBundleIdentifier</key><string>tf.trom.legatorocr</string>
  <key>CFBundleExecutable</key><string>legatorocr-gui</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>NSHighResolutionCapable</key><true/>
</dict></plist>
PLIST
    ;;
  *windows*)
    cp "$BIN/legatorocr-gui.exe" "$OUT/"
    ;;
  *)
    cp "$BIN/legatorocr-gui" "$OUT/"
    cat > "$OUT/legatorocr.desktop" <<DESK
[Desktop Entry]
Type=Application
Name=LegatorOCR
Comment=Scan post-processing: split, deskew, crop, PDF with OCR
Exec=legatorocr-gui
Terminal=false
Categories=Graphics;Office;
DESK
    ;;
esac

cd dist
case "$TARGET" in
  *windows*) if command -v 7z >/dev/null; then 7z a -tzip "${NAME}.zip" "${NAME}" >/dev/null; else zip -qr "${NAME}.zip" "${NAME}"; fi; ARCHIVE="${NAME}.zip" ;;
  *apple*)   zip -qr "${NAME}.zip" "${NAME}"; ARCHIVE="${NAME}.zip" ;;
  *)         tar -czf "${NAME}.tar.gz" "${NAME}"; ARCHIVE="${NAME}.tar.gz" ;;
esac
if command -v sha256sum >/dev/null; then sha256sum "$ARCHIVE" > "${ARCHIVE}.sha256"; else shasum -a 256 "$ARCHIVE" > "${ARCHIVE}.sha256"; fi
echo "packaged: dist/${ARCHIVE}"
