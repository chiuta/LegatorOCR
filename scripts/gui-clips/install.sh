#!/bin/sh
# Copies the filmed clips (mp4) into docs/video/gui and makes the 640x405 poster of each.
# Usage: scripts/gui-clips/install.sh [DIR_WITH_mp4] c02 c03 ...
src=${1:-/tmp/gui/out}; shift
dst=$(dirname "$0")/../../docs/video/gui
for c in "$@"; do
  cp "$src/$c.mp4" "$dst/$c.mp4"
  ffmpeg -y -loglevel error -ss "${POSTER_AT:-6}" -i "$dst/$c.mp4" -frames:v 1 -vf scale=640:405 -q:v 4 "$dst/$c.jpg"
  echo "installed $c"
done
