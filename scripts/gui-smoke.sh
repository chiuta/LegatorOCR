#!/bin/sh
# Drives the real window under a virtual display and checks what it draws.
#   scripts/gui-smoke.sh target/debug/legatorocr-gui IMAGE.jpg
# Needs xvfb-run, xdotool, ImageMagick `import`, python3 with Pillow and numpy, and Mesa software
# GL. Coordinates assume a 1400x900 screen and the default English window; they were taken from
# screenshots of version 0.2.0-pre.16 and break when the layout changes. Checks: the welcome card
# is shown without pages; with a page loaded the preview appears; tracing a guide line draws its
# green nodes; a double click on a node removes it; the program is still running at the end.
set -e
GUI=$(realpath "$1"); IMG=$(realpath "$2"); OUT=${3:-/tmp/gui-smoke}
mkdir -p "$OUT"
cat > "$OUT/run.sh" <<EOF
#!/bin/sh
export LIBGL_ALWAYS_SOFTWARE=1 HOME=$OUT/home
mkdir -p \$HOME; rm -rf \$HOME/.config \$HOME/.local
c(){ xdotool mousemove \$1 \$2 click 1; sleep \${3:-1.5}; }
"$GUI" > $OUT/empty.log 2>&1 &
P=\$!; sleep 10; import -window root $OUT/welcome.png; kill \$P; sleep 1
"$GUI" "$IMG" > $OUT/gui.log 2>&1 &
P=\$!; sleep 10
c 80 117 6; import -window root $OUT/preview.png
c 558 43 2; c 274 66 2
xdotool mousemove 265 192; xdotool mousedown 1
for p in "300 191" "340 189" "380 188" "420 188" "460 189" "500 191" "540 192" "575 193"; do xdotool mousemove \$p; sleep 0.15; done
xdotool mouseup 1; sleep 6; import -window root $OUT/guide.png
xdotool mousemove 300 191; sleep 0.5; xdotool click --repeat 2 --delay 90 1; sleep 5
import -window root $OUT/node_removed.png
kill -0 \$P && echo ALIVE > $OUT/alive; kill \$P
EOF
chmod +x "$OUT/run.sh"
xvfb-run -a -s "-screen 0 1400x900x24" "$OUT/run.sh"
python3 - "$OUT" <<'PY'
import sys
from PIL import Image
import numpy as np
o = sys.argv[1]
def px(n): return np.array(Image.open(f"{o}/{n}.png").convert("RGB")).astype(int)
def green(a, box):
    x0, y0, x1, y1 = box
    s = a[y0:y1, x0:x1]
    return int(((s[..., 1] > 150) & (s[..., 0] < 90) & (s[..., 2] < 120)).sum())
bad = 0
def check(name, ok):
    global bad
    print(("ok   " if ok else "FAIL ") + name); bad += not ok
w = px("welcome")
check("welcome card: bright text in the middle", int((w[90:290, 300:900].sum(axis=2) > 500).sum()) > 400)
p = px("preview")
check("preview: two page images (white areas) in the middle", int((p[150:650, 238:1000].sum(axis=2) > 740).sum()) > 60000)
g = green(px("guide"), (238, 150, 610, 230))
r = green(px("node_removed"), (238, 150, 610, 230))
check(f"guide line traced: {g} green px", g > 300)
check(f"node removed: green {g} -> {r}", r < g)
check("program still running", open(f"{o}/alive").read().strip() == "ALIVE")
sys.exit(1 if bad else 0)
PY
