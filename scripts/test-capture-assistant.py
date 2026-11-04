#!/usr/bin/env python3
"""Synthetic end-to-end test of web/capture/index.html in headless Chromium with a fake camera.

usage: scripts/test-capture-assistant.py [PAGE_IMAGE ...] [--cli PATH_TO_legatorocr]
needs: python3 playwright (+ its Chromium), cv2, numpy, ffmpeg.  Everything here is SYNTHETIC:
a scanned page photographed by nothing, pasted on a dark background; no real phone, lens,
shadow or motion blur is involved.  Prints the measured corner error (px at 640 wide)."""
import sys, os, subprocess, tempfile, zipfile, math, time, pathlib, json
import cv2, numpy as np
from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parent.parent
HTML = ROOT / "web" / "capture" / "index.html"
args = sys.argv[1:]
cli = "legatorocr"
if "--cli" in args:
    i = args.index("--cli"); cli = args[i + 1]; del args[i:i + 2]
pages = args or ["/tmp/ocr_eval/img/p7.png", "/tmp/ocr_eval/img/p13.png", "/tmp/ocr_eval/img/p19.png"]
W, H = 960, 1280  # camera frame (portrait, like a phone); the page analyses at 640 x 853
tmp = pathlib.Path(tempfile.mkdtemp(prefix="capt-"))


def scene(page_png, cx, cy, width, angle_deg, persp=0.0, bg=55):
    pg = cv2.imread(page_png)
    ph, pw = pg.shape[:2]
    h = width * ph / pw
    c = np.array([[-width / 2, -h / 2], [width / 2, -h / 2], [width / 2, h / 2], [-width / 2, h / 2]], np.float32)
    a = math.radians(angle_deg)
    R = np.array([[math.cos(a), -math.sin(a)], [math.sin(a), math.cos(a)]], np.float32)
    dst = c @ R.T + np.array([cx, cy], np.float32)
    dst[0, 0] += persp * width; dst[1, 0] -= persp * width  # mild keystone
    src = np.array([[0, 0], [pw, 0], [pw, ph], [0, ph]], np.float32)
    M = cv2.getPerspectiveTransform(src, dst)
    yy, xx = np.mgrid[0:H, 0:W]
    canvas = np.full((H, W, 3), bg, np.float32) + (xx / W * 14 + yy / H * 10)[..., None]  # gradient desk
    canvas += np.random.default_rng(1).normal(0, 2, canvas.shape)
    warped = cv2.warpPerspective(pg, M, (W, H), flags=cv2.INTER_AREA, borderMode=cv2.BORDER_TRANSPARENT, dst=canvas.astype(np.uint8).copy())
    return warped, dst


def tweak(img, kind):
    if kind == "blur": return cv2.GaussianBlur(img, (0, 0), 7)
    if kind == "dark": return (img.astype(np.float32) * 0.28).astype(np.uint8)
    if kind == "bright": return np.clip(img.astype(np.float32) * 1.0 + 60, 0, 255).astype(np.uint8)
    return img


def to_y4m(img, name):
    png = tmp / (name + ".png"); cv2.imwrite(str(png), img)
    out = tmp / (name + ".y4m")
    subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-loop", "1", "-framerate", "10", "-i", str(png), "-t", "2", "-pix_fmt", "yuv420p", str(out)], check=True)
    return out


def order(c):  # TL, TR, BR, BL by the same extreme rule the app uses
    c = np.asarray(c, float); s = c.sum(1); d = c[:, 0] - c[:, 1]
    return np.array([c[s.argmin()], c[d.argmax()], c[s.argmax()], c[d.argmin()]])


def run(browser_args, y4m, wait_ms, action=None, downloads=None):
    with sync_playwright() as p:
        b = p.chromium.launch(args=["--use-fake-ui-for-media-stream", "--use-fake-device-for-media-stream", f"--use-file-for-fake-video-capture={y4m}"] + browser_args)
        ctx = b.new_context(permissions=["camera"], accept_downloads=True)
        pg = ctx.new_page(); errs = []
        pg.on("pageerror", lambda e: errs.append(str(e)))
        pg.on("request", lambda r: errs.append("EXTERNAL REQUEST " + r.url) if not r.url.startswith(("file:", "blob:", "data:")) else None)
        pg.goto(HTML.as_uri()); pg.click("#start")
        pg.wait_for_timeout(wait_ms)
        res = action(pg) if action else None
        state = pg.evaluate("({s: document.getElementById('reason').dataset.state, c: window.__cap.corners, a: window.__cap.auto, lap: window.__cap.lap, mean: window.__cap.mean, ah: window.__cap.ah, n: document.querySelectorAll('#strip .th').length})")
        b.close()
        return state, res, errs


results = {"errors_px": [], "fail": []}
def check(cond, msg):
    print(("PASS " if cond else "FAIL ") + msg)
    if not cond: results["fail"].append(msg)

# --- 1. detection error on >= 5 placements (shift / rotate / keystone / other pages) ---
placements = [
    (pages[0], W * .50, H * .50, 560, 0, 0),
    (pages[0], W * .42, H * .46, 520, 8, 0),
    (pages[1 % len(pages)], W * .55, H * .52, 540, -14, 0),
    (pages[2 % len(pages)], W * .48, H * .55, 500, 22, 0),
    (pages[0], W * .52, H * .44, 530, -4, 0.02),
    (pages[1 % len(pages)], W * .47, H * .50, 480, 31, 0),
]
for k, (pgimg, cx, cy, wd, ang, pe) in enumerate(placements):
    img, dst = scene(pgimg, cx, cy, wd, ang, pe)
    st, _, errs = run([], to_y4m(img, f"pl{k}"), 2500)
    if st["c"] is None:
        check(False, f"placement {k}: no outline found (state {st['s']})"); continue
    gt = order(dst) * (640 / W); det = order(st["c"])
    e = np.linalg.norm(gt - det, axis=1)
    results["errors_px"].append(e.max())
    print(f"placement {k}: rot {ang:+d} deg, width {wd}px -> corner error px@640 per corner {np.round(e, 1).tolist()} max {e.max():.1f}; state={st['s']}; lap={st['lap']:.0f} mean={st['mean']:.0f}")
    check(not errs, f"placement {k}: no page errors / external requests {errs}")
if results["errors_px"]:
    print(f"detection error px@640: mean {np.mean(results['errors_px']):.1f}, max {np.max(results['errors_px']):.1f} over {len(results['errors_px'])} placements")
    check(max(results["errors_px"]) < 12, "max corner error below 12 px at 640 wide")

# --- 2. refusals, each with the right reason ---
base, _ = scene(pages[0], W * .5, H * .5, 560, 3)
cases = [("blur", tweak(base, "blur"), "too-blurry"), ("dark", tweak(base, "dark"), "too-dark"),
         ("bright", tweak(scene(pages[0], W * .5, H * .5, 560, 3, bg=235)[0], "bright"), None)]
shifted, _ = scene(pages[0], W * .78, H * .5, 560, 3)  # right part of the page is outside the frame
cases.append(("cut", shifted, "not-full"))
cases.append(("nopage", np.full((H, W, 3), 60, np.uint8), "no-page"))
for sg in (1.5, 3.0):  # information only: where the untuned threshold (60) sits for milder blur
    cases.append((f"blur-sigma{sg}", cv2.GaussianBlur(base, (0, 0), sg), None))
for name, img, want in cases:
    st, _, _ = run([], to_y4m(img, "r_" + name), 5000)
    if want:
        check(st["s"] == want and st["a"] == 0, f"refusal '{name}': state={st['s']} (want {want}), auto-captures={st['a']}, lap={st['lap']}, mean={st['mean']}")
    else:
        print(f"info '{name}' (no pass/fail wanted): state={st['s']} auto={st['a']} lap={st['lap']}")

# --- 3. good page: auto capture fires exactly once; manual shot of a blurred frame; ZIP ---
good, _ = scene(pages[0], W * .5, H * .5, 560, 2)
dl_dir = tmp / "dl"; dl_dir.mkdir()
def act(pg):
    pg.wait_for_function("window.__cap.auto >= 1", timeout=15000)
    pg.wait_for_timeout(6000)  # must NOT shoot the same page again
    pg.click("#shoot")  # a manual second page
    pg.wait_for_function("document.querySelectorAll('#strip .th').length >= 2", timeout=5000)
    pg.click("#shoot"); pg.wait_for_function("document.querySelectorAll('#strip .th').length >= 3", timeout=5000)
    pg.click("#strip .th:nth-child(2) button")  # delete the 2nd thumbnail
    with pg.expect_download() as d:
        pg.click("#save")
    path = dl_dir / d.value.suggested_filename; d.value.save_as(path); return path
st, zpath, errs = run([], to_y4m(good, "good"), 1500, act)
check(not errs, f"good page: no page errors / external requests {errs}")
check(st["a"] == 1 and st["n"] == 2, f"auto fired once (auto={st['a']}), strip has {st['n']} thumbnails after one deletion (want 2)")
import re
check(bool(re.fullmatch(r"burst-\d{8}-\d{6}\.zip", zpath.name)), f"zip name {zpath.name}")
out = tmp / "unz"; out.mkdir()
with zipfile.ZipFile(zpath) as z:
    check(z.testzip() is None, "zip CRC ok (python zipfile.testzip)")
    names = z.namelist(); z.extractall(out)
    check(all(i.compress_type == 0 for i in z.infolist()), "entries are stored (method 0)")
check(names == ["page-0001.jpg", "page-0002.jpg"], f"names {names}")
sizes = [cv2.imread(str(out / n)).shape[:2] for n in names]
print("jpeg sizes (h,w):", sizes, "= camera resolution of the fake device:", (H, W))
check(all(s == (H, W) for s in sizes), "JPEGs have the full camera resolution")
r = subprocess.run(["unzip", "-tq", str(zpath)], capture_output=True, text=True)
print("unzip -t:", r.stdout.strip(), r.stderr.strip())
check(r.returncode == 0, "info-zip `unzip -t` accepts the archive")
# --- 4. legatorocr --burst accepts it ---
pdf = tmp / "o.pdf"
r = subprocess.run([cli, "--burst", "--pdf", str(pdf), "-o", str(tmp / "cliout"), *[str(out / n) for n in names]], capture_output=True, text=True)
print("legatorocr rc", r.returncode, "\n" + (r.stderr or "")[-600:])
check(r.returncode == 0 and pdf.exists(), "legatorocr --burst processed the unzipped pages to a PDF")
check("left out" not in r.stderr, "--burst did not drop any page as a duplicate")

print("\nFAILED: " + "; ".join(results["fail"]) if results["fail"] else "\nall checks passed (SYNTHETIC frames only)")
sys.exit(1 if results["fail"] else 0)
