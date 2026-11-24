"""Run one clip script: python3 runclip.py c08   (clips/c08.py defines DURATION, FILES, act(tl))."""
import sys, os, time, subprocess, pathlib, importlib, shutil
sys.path.insert(0, str(pathlib.Path(__file__).parent))
import drv
name = sys.argv[1]
mod = importlib.import_module("clips." + name)
GUI = os.environ.get("GUI", "/tmp/target/debug/legatorocr-gui")
SC = pathlib.Path(os.environ.get("SCANS", "/tmp/gui/scans"))
OUT = pathlib.Path(os.environ.get("CLIPS_OUT", "/tmp/gui/out")); OUT.mkdir(parents=True, exist_ok=True)
home = pathlib.Path("/tmp/gui/home-" + name); shutil.rmtree(home, ignore_errors=True); home.mkdir(parents=True)
env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE="1", HOME=str(home), DISPLAY=os.environ.get("DISPLAY", ":99"))
files = [str(SC / f) for f in getattr(mod, "FILES", [])]
p = subprocess.Popen([GUI] + files, env=env, stdout=open(OUT / f"{name}.log", "w"), stderr=subprocess.STDOUT)
time.sleep(float(getattr(mod, "WARMUP", 10)))
drv.focus_app()
raw = OUT / f"{name}.raw.mkv"
rec = drv.start_recording(raw); time.sleep(0.8)
tl = drv.Timeline()
try:
    mod.act(tl, drv, SC)
    tl.at(mod.DURATION)
finally:
    drv.stop_recording(rec)
    p.terminate()
    try: p.wait(10)
    except Exception: p.kill()
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-ss", "0.8", "-t", str(mod.DURATION), "-i", str(raw), "-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(OUT / f"{name}.mp4")], check=True)
raw.unlink()
print("done", name, round(tl.now(), 1), "s")
