#!/bin/sh
# Builds a binary .deb straight from target/release/legatorocr (no debhelper needed).
# usage: packaging/debian/build-deb.sh VERSION OUTDIR   e.g. 0.2.0~pre.15 /tmp
set -e
V=${1:?version}; OUT=${2:-.}; BIN=target/release/legatorocr
R=$(mktemp -d); trap 'rm -rf "$R"' EXIT
install -Dm755 "$BIN" "$R/usr/bin/legatorocr"
strip "$R/usr/bin/legatorocr" 2>/dev/null || true
install -Dm644 README.md "$R/usr/share/doc/legatorocr/README.md"
mkdir -p "$R/DEBIAN"
SIZE=$(du -sk "$R/usr" | cut -f1)
cat > "$R/DEBIAN/control" <<C
Package: legatorocr
Version: $V
Section: graphics
Priority: optional
Architecture: $(dpkg --print-architecture)
Maintainer: Alexandru-Ionut Chiuta <alexio@trom.tf>
Installed-Size: $SIZE
Depends: libc6, libgcc-s1
Recommends: tesseract-ocr
Homepage: https://github.com/chiuta/LegatorOCR
Description: scan post-processing, PDF export and OCR
 Clean-room command-line tool: deskew, page split, content selection,
 binarisation, PDF export and OCR.
C
dpkg-deb --root-owner-group --build "$R" "$OUT/legatorocr_${V}_$(dpkg --print-architecture).deb"
