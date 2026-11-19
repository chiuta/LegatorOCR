#!/bin/sh
# Filmează ecranul X11 (1360x860, 25 fps) pentru un clip GUI. Folosire: ./record-gui-clip.sh c03
# Pornește întâi LegatorOCR cu fereastra în colțul din stânga-sus (0,0), dimensiune 1360x860.
# Oprire: apasă q în terminal. Rezultat: cNN.raw.mkv (apoi comprimă cu comanda din REFILMARE.md).
set -e
[ -n "$1" ] || { echo "folosire: $0 c03"; exit 1; }
exec ffmpeg -f x11grab -framerate 25 -video_size 1360x860 -i "${DISPLAY:-:0}+0,0" -c:v libx264 -preset ultrafast -crf 18 "$1.raw.mkv"
