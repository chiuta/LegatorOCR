"""Tiny X11 driver for filming the LegatorOCR window under Xvfb (python-xlib XTEST, no xdotool)."""
import os, subprocess, time, pathlib
from Xlib import X, XK, display
from Xlib.ext import xtest

os.environ.setdefault("DISPLAY", ":99")
D = display.Display(os.environ.get("DISPLAY", ":99"))
ROOT = D.screen().root

def sync(): D.sync()

def move(x, y):
    xtest.fake_input(D, X.MotionNotify, x=int(x), y=int(y)); sync()

def glide(x, y, steps=12, dt=0.02):
    p = ROOT.query_pointer(); x0, y0 = p.root_x, p.root_y
    for i in range(1, steps + 1):
        move(x0 + (x - x0) * i / steps, y0 + (y - y0) * i / steps); time.sleep(dt)

def click(x=None, y=None, button=1, double=False, glide_to=True):
    if x is not None:
        glide(x, y) if glide_to else move(x, y)
    for _ in range(2 if double else 1):
        xtest.fake_input(D, X.ButtonPress, button); sync(); time.sleep(0.06)
        xtest.fake_input(D, X.ButtonRelease, button); sync(); time.sleep(0.08)

def drag(x0, y0, x1, y1, steps=20, dt=0.03, button=1):
    glide(x0, y0, 6); xtest.fake_input(D, X.ButtonPress, button); sync(); time.sleep(0.15)
    for i in range(1, steps + 1):
        move(x0 + (x1 - x0) * i / steps, y0 + (y1 - y0) * i / steps); time.sleep(dt)
    time.sleep(0.15); xtest.fake_input(D, X.ButtonRelease, button); sync()

def _kc(name):
    ks = XK.string_to_keysym(name)
    return D.keysym_to_keycode(ks), ks

def key(name, mods=()):
    kc, _ = _kc(name)
    mk = [D.keysym_to_keycode(XK.string_to_keysym(m)) for m in mods]
    for m in mk: xtest.fake_input(D, X.KeyPress, m)
    xtest.fake_input(D, X.KeyPress, kc); sync(); time.sleep(0.05)
    xtest.fake_input(D, X.KeyRelease, kc)
    for m in reversed(mk): xtest.fake_input(D, X.KeyRelease, m)
    sync(); time.sleep(0.08)

def type_text(s, dt=0.05):
    for ch in s:
        if ch == " ": key("space")
        elif ch == "/": key("slash")
        elif ch == ".": key("period")
        elif ch == "-": key("minus")
        elif ch == "_": key("underscore", ["Shift_L"])
        elif ch == ",": key("comma")
        elif ch.isupper(): key(ch, ["Shift_L"])
        else: key(ch)
        time.sleep(dt)

def scroll(x, y, clicks, down=True):
    glide(x, y, 6)
    for _ in range(abs(clicks)):
        b = 5 if down else 4
        xtest.fake_input(D, X.ButtonPress, b); sync(); xtest.fake_input(D, X.ButtonRelease, b); sync(); time.sleep(0.08)

def shot(path):
    subprocess.run(["import", "-window", "root", str(path)], check=True)

class Timeline:
    """Run actions at given seconds from the start of the recording (the cue times of the subtitles)."""
    def __init__(self): self.t0 = time.time()
    def at(self, t):
        d = t - (time.time() - self.t0)
        if d > 0: time.sleep(d)
    def now(self): return time.time() - self.t0

def start_recording(out, fps=25):
    return subprocess.Popen(["ffmpeg", "-y", "-loglevel", "error", "-f", "x11grab", "-framerate", str(fps), "-video_size", "1360x860",
        "-i", os.environ.get("DISPLAY", ":99") + "+0,0", "-c:v", "libx264", "-preset", "ultrafast", "-crf", "20", str(out)], stdin=subprocess.PIPE)

def stop_recording(p):
    p.stdin.write(b"q"); p.stdin.flush(); p.wait(timeout=60)

def focus_app():
    """No window manager under Xvfb: give the program's window the keyboard focus ourselves."""
    def walk(w, depth=0):
        try:
            for c in w.query_tree().children:
                n = c.get_wm_name() or ""
                if "egal" in n or "LegatorOCR" in n or "legatorocr" in (c.get_wm_class() or ("",))[0].lower():
                    return c
                r = walk(c, depth + 1) if depth < 1 else None
                if r: return r
        except Exception:
            pass
        return None
    w = walk(ROOT)
    if w:
        w.set_input_focus(X.RevertToParent, X.CurrentTime); sync()
    return w
