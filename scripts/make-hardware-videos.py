#!/usr/bin/env python3
"""Makes the two hardware clips (docs/video/hardware/*.mp4 + one .vtt per language) from the author's
diagram and photos (docs/guide-src/img/hardware) and the texts of docs/guide-src/hardware/content.py.
Needs Pillow and ffmpeg.   Usage: python3 scripts/make-hardware-videos.py"""
import math, pathlib, subprocess, sys
from PIL import Image, ImageDraw, ImageFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "docs/guide-src/hardware"))
from content import T, LANGS  # noqa: E402

IMG = ROOT / "docs/guide-src/img/hardware"
OUT = ROOT / "docs/video/hardware"
OUT.mkdir(parents=True, exist_ok=True)
W, H, FPS = 1280, 720, 25
BG = (20, 24, 31)
ORANGE = (255, 170, 30)
BLUE = (29, 79, 145)
FONT = next((p for p in ("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
                         "/usr/share/fonts/dejavu/DejaVuSans-Bold.ttf") if pathlib.Path(p).exists()), None)
font = lambda n: ImageFont.truetype(FONT, n) if FONT else ImageFont.load_default()

diagram = Image.open(IMG / "diagram.png").convert("RGB")
photos = {k: Image.open(IMG / f"{k}.jpg").convert("RGB") for k in ("phone-screen", "phone-edge", "operator")}


def vtt_time(t):
    return f"{int(t // 3600):02d}:{int(t % 3600 // 60):02d}:{t % 60:06.3f}"


def write_vtt(name, lang, cues):
    lines = ["WEBVTT", ""]
    for a, b, text in cues:
        lines += [f"{vtt_time(a)} --> {vtt_time(b)}", text, ""]
    (OUT / f"{name}.{lang}.vtt").write_text("\n".join(lines), encoding="utf-8")


def badge(d, cx, cy, n, r=22, fill=BLUE, txt=(255, 255, 255)):
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=fill, outline=(255, 255, 255), width=2)
    d.text((cx, cy), str(n), font=font(int(r * 1.1)), fill=txt, anchor="mm")


def encode(name, nframes, frame_fn, cues_by_lang, duration):
    for lang in LANGS:
        write_vtt(name, lang, cues_by_lang[lang])
    raw = OUT / f"{name}.raw.mp4"
    p = subprocess.Popen(["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "rgb24", "-s", f"{W}x{H}",
                          "-r", str(FPS), "-i", "-", "-an", "-c:v", "libx264", "-preset", "veryfast", "-crf", "26",
                          "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(raw)], stdin=subprocess.PIPE)
    for i in range(nframes):
        p.stdin.write(frame_fn(i / FPS).tobytes())
    p.stdin.close()
    p.wait()
    # soft subtitles for all eight languages inside the mp4
    cmd = ["ffmpeg", "-y", "-loglevel", "error", "-i", str(raw)]
    for lang in LANGS:
        cmd += ["-i", str(OUT / f"{name}.{lang}.vtt")]
    cmd += ["-map", "0:v"]
    for i in range(len(LANGS)):
        cmd += ["-map", str(i + 1)]
    cmd += ["-c:v", "copy", "-c:s", "mov_text"]
    for i, lang in enumerate(LANGS):
        cmd += [f"-metadata:s:s:{i}", f"language={ {'ro':'ron','en':'eng','fr':'fra','it':'ita','es':'spa','pt':'por','de':'deu','hu':'hun'}[lang] }"]
    cmd += [str(OUT / f"{name}.mp4")]
    subprocess.run(cmd, check=True)
    raw.unlink()


def fit(img, w, h):
    k = min(w / img.width, h / img.height)
    return img.resize((max(1, int(img.width * k)), max(1, int(img.height * k))), Image.LANCZOS)


def title_card(t, text, big=44):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    words, lines, cur = text.split(), [], ""
    for w in words:
        if d.textlength((cur + " " + w).strip(), font=font(big)) > W - 160:
            lines.append(cur)
            cur = w
        else:
            cur = (cur + " " + w).strip()
    lines.append(cur)
    y = H // 2 - len(lines) * big // 2
    for ln in lines:
        d.text((W // 2, y), ln, font=font(big), fill=(255, 255, 255), anchor="mm")
        y += int(big * 1.3)
    return im


# ------------------------------------------------------------------ clip 1: the work table
REG = [  # x, y, w, h on the 1911x1154 diagram: phone, book stool, operator's chair, lamp, remote (neck)
    (610, 455, 260, 70), (690, 640, 360, 440), (1135, 545, 345, 545), (680, 495, 140, 130), (1230, 350, 230, 230)]
STEP1 = 8.0
INTRO1, END1 = 4.0, 4.0
ORDER1 = [0, 1, 2, 3, 4]
dh = 660
dw = int(diagram.width * dh / diagram.height)
dia = diagram.resize((dw, dh), Image.LANCZOS)
sx = dw / diagram.width
ox, oy = (W - dw) // 2, 30


def frame1(t):
    if t < INTRO1:
        im = Image.new("RGB", (W, H), BG)
        a = min(1.0, t / 1.5)
        im.paste(Image.blend(Image.new("RGB", dia.size, BG), dia, a), (ox, oy))
        return im
    k = int((t - INTRO1) // STEP1)
    im = Image.new("RGB", (W, H), BG)
    if k >= len(ORDER1):
        im.paste(dia, (ox, oy))
        d = ImageDraw.Draw(im)
        a = 0.5 + 0.5 * math.sin(t * 5)
        d.rectangle((ox + 1487 * sx, oy + 30 * sx, ox + 1880 * sx, oy + 990 * sx), outline=ORANGE, width=3 + int(2 * a))
        return im
    dim = Image.blend(dia, Image.new("RGB", dia.size, BG), 0.62)
    x, y, w, h = [int(v * sx) for v in REG[ORDER1[k]]]
    dim.paste(dia.crop((x, y, x + w, y + h)), (x, y))
    im.paste(dim, (ox, oy))
    d = ImageDraw.Draw(im)
    pulse = 0.5 + 0.5 * math.sin((t - INTRO1) * 6)
    d.rectangle((ox + x - 4, oy + y - 4, ox + x + w + 4, oy + y + h + 4), outline=ORANGE, width=3 + int(3 * pulse))
    badge(d, ox + x - 4, oy + y - 4, k + 1, r=26, fill=(210, 120, 0))
    # progress dots
    for i in range(5):
        badge(d, W // 2 - 80 + i * 40, H - 26, i + 1, r=13, fill=(210, 120, 0) if i == k else (70, 80, 96))
    return im


# ------------------------------------------------------------------ clip 2: taking the photos
STEP2, INTRO2 = 7.0, 4.0
SRC2 = ["phone-screen", "operator", "phone-edge", "phone-edge", "phone-edge", "phone-screen", "diagram"]


def panel(key, t):
    base = diagram.crop((0, 90, 560, 1100)) if key == "diagram" else photos[key]
    scale = 1.0 + 0.06 * (t % STEP2) / STEP2
    f = fit(base, 760, 620)
    f = f.resize((int(f.width * scale), int(f.height * scale)), Image.LANCZOS)
    box = Image.new("RGB", (760, 620), BG)
    box.paste(f, ((760 - f.width) // 2, (620 - f.height) // 2))
    return box


def ring(d, cx, cy, r, frac, label):
    d.ellipse((cx - r, cy - r, cx + r, cy + r), outline=(70, 80, 96), width=10)
    d.arc((cx - r, cy - r, cx + r, cy + r), -90, -90 + 360 * frac, fill=ORANGE, width=10)
    d.text((cx, cy), label, font=font(int(r * 0.9)), fill=(255, 255, 255), anchor="mm")


def frame2(t):
    im = Image.new("RGB", (W, H), BG)
    d = ImageDraw.Draw(im)
    k = 0 if t < INTRO2 else min(6, int((t - INTRO2) // STEP2))
    tl = (t - INTRO2) % STEP2 if t >= INTRO2 else 0
    im.paste(panel(SRC2[k], tl), (50, 50))
    d.rectangle((50, 50, 810, 670), outline=(90, 100, 118), width=2)
    for i in range(7):  # the seven steps, numbers only (the words are in the subtitles)
        cy = 90 + i * 85
        on = i == k and t >= INTRO2
        badge(d, 1000, cy, i + 1, r=26 if on else 20, fill=(210, 120, 0) if on else (70, 80, 96))
        if on:
            d.rounded_rectangle((950 - 20, cy - 40, 1050 + 20, cy + 40), radius=14, outline=ORANGE, width=3)
    if k == 3 and t >= INTRO2:  # step 4: the 10-second burst timer, then 5 s
        span = 5.0 if tl >= 3.5 else 10.0
        sec = (tl * (10.0 / 3.5) if tl < 3.5 else (tl - 3.5) * (5.0 / 3.5)) % span if span else 0
        left = span - sec
        ring(d, 1160, 380, 70, left / span, f"{int(math.ceil(left))}s")
    return im


def cues(L, steps, attn, step_len, intro_len, title):
    t = T[L]
    c = [(0.3, intro_len - 0.3, t[title])]
    for i, s in enumerate(steps):
        a = intro_len + i * step_len
        c.append((a + 0.2, a + step_len - 0.2, f"{i + 1}. {s}"))
    a = intro_len + len(steps) * step_len
    return c, a


TITLES1 = {L: T[L]["h_rig"].split(". ", 1)[-1] for L in LANGS}
TITLES2 = {L: T[L]["h_shoot"].split(". ", 1)[-1] for L in LANGS}

c1 = {}
for L in LANGS:
    c, a = cues(L, T[L]["rig_steps"], T[L]["rig_attn"], STEP1, INTRO1, "h_rig")
    c[0] = (0.3, INTRO1 - 0.3, TITLES1[L])
    c.append((a + 0.2, a + END1 + 4.0, T[L]["rig_attn"]))
    c1[L] = c
dur1 = INTRO1 + 5 * STEP1 + END1 + 4.0
encode("h01-work-table", int(dur1 * FPS), frame1, c1, dur1)

c2 = {}
for L in LANGS:
    c, a = cues(L, T[L]["shoot_steps"], T[L]["shoot_attn"], STEP2, INTRO2, "h_shoot")
    c[0] = (0.3, INTRO2 - 0.3, TITLES2[L])
    c.append((a + 0.2, a + 7.0, T[L]["shoot_attn"]))
    c2[L] = c
dur2 = INTRO2 + 7 * STEP2 + 7.0


def frame2_wrap(t):
    return frame2(min(t, INTRO2 + 7 * STEP2 - 0.01)) if t < INTRO2 + 7 * STEP2 else frame2(INTRO2 + 7 * STEP2 - 0.01)


encode("h02-taking-photos", int(dur2 * FPS), frame2_wrap, c2, dur2)
print("done", dur1, dur2)
