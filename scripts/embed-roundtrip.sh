#!/bin/sh
# Round trip of --embed-project on real pages: process -> embed -> extract -> process again.
# Usage: scripts/embed-roundtrip.sh PAGE.png [PAGE2.png ...]   (env LEGATOROCR=path/to/binary, extra options in OPTS)
# Prints the sizes (plain PDF, PDF with project, originals) and whether page images and PDFs are identical.
set -eu
BIN=$(realpath "${LEGATOROCR:-target/release/legatorocr}")
W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
export SOURCE_DATE_EPOCH=1700000000
mkdir -p "$W/a" "$W/b" "$W/c"
# shellcheck disable=SC2086
"$BIN" "$@" --pdf "$W/a/o.pdf" -o "$W/a/pages" --format png ${OPTS:-} >/dev/null 2>&1
# shellcheck disable=SC2086
"$BIN" "$@" --pdf "$W/b/o.pdf" -o "$W/b/pages" --format png --embed-project ${OPTS:-} >/dev/null 2>&1
"$BIN" --extract-project "$W/b/o.pdf" "$W/x" >/dev/null
mkdir -p "$W/x/c"
(cd "$W/x" && "$BIN" --project project.lgr --pdf c/o.pdf -o c/pages --format png >/dev/null 2>&1)
orig=0; for f in "$@"; do orig=$((orig + $(wc -c < "$f"))); done
plain=$(wc -c < "$W/a/o.pdf"); emb=$(wc -c < "$W/b/o.pdf")
echo "originals $orig bytes; plain PDF $plain; PDF with project $emb; overhead $((emb - plain)) bytes ($(( (emb - plain) * 1000 / plain ))/1000 of the plain PDF)"
same=1
for f in "$W"/b/pages/*.png; do
  n=$(basename "$f"); m=$(ls "$W"/x/c/pages | grep -F "_${n#}" | head -1 || true)
  [ -n "$m" ] || m=$(ls "$W"/x/c/pages | sed -n "$(ls "$W/b/pages" | grep -n -F "$n" | cut -d: -f1)p")
  cmp -s "$f" "$W/x/c/pages/$m" || { echo "page differs: $n"; same=0; }
done
[ "$same" = 1 ] && echo "page images: byte-identical ($(ls "$W/b/pages" | wc -l) pages)"
if cmp -s "$W/a/o.pdf" "$W/x/c/o.pdf"; then echo "re-processed PDF == first plain PDF: byte-identical"; else echo "re-processed PDF differs from the first plain PDF"; exit 1; fi
