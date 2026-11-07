#!/usr/bin/env python3
"""Runs `--scan-escl` against a small fake eSCL scanner on 127.0.0.1 and checks the pages it saves.
Covers --scan-dir, --scan-dpi, --scan-mode, --scan-source, --scan-pages and --scan-only, and that
the job request carries the dpi and colour mode asked for. It proves the protocol steps and the
options, not any real scanner (none was available).
Usage: scripts/test-escl.py [path/to/legatorocr]"""
import http.server, io, pathlib, re, subprocess, sys, tempfile, threading
from PIL import Image

root = pathlib.Path(__file__).resolve().parent.parent
exe = sys.argv[1] if len(sys.argv) > 1 else str(root / "target/release/legatorocr")

def jpeg(level):
    b = io.BytesIO()
    Image.new("L", (200, 280), level).save(b, "JPEG")
    return b.getvalue()

class Scanner(http.server.BaseHTTPRequestHandler):
    pages = 3          # sheets in the feeder
    served = 0
    jobs = []          # the XML of every job request
    def log_message(self, *a): pass
    def send(self, code, body=b"", headers=()):
        self.send_response(code)
        for k, v in headers: self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)
    def do_GET(self):
        if self.path.endswith("/ScannerCapabilities"):
            self.send(200, b"<scan:ScannerCapabilities><scan:Platen><scan:MaxWidth>2550</scan:MaxWidth>"
                           b"<scan:MaxHeight>3507</scan:MaxHeight></scan:Platen></scan:ScannerCapabilities>")
        elif self.path.endswith("/NextDocument"):
            if Scanner.served < Scanner.pages:
                Scanner.served += 1
                self.send(200, jpeg(40 * Scanner.served), [("Content-Type", "image/jpeg")])
            else:
                self.send(404)
        else:
            self.send(404)
    def do_POST(self):
        n = int(self.headers.get("Content-Length", 0))
        Scanner.jobs.append(self.rfile.read(n).decode())
        Scanner.served = 0
        self.send(201, headers=[("Location", "/eSCL/ScanJobs/7")])
    def do_DELETE(self):
        self.send(200)

srv = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Scanner)
threading.Thread(target=srv.serve_forever, daemon=True).start()
target = f"127.0.0.1:{srv.server_address[1]}"
fail = []
def run(name, args, want_files, want_job=()):
    d = pathlib.Path(tempfile.mkdtemp())
    Scanner.jobs.clear()
    r = subprocess.run([exe, "--scan-escl", target, "--scan-dir", str(d / "s")] + args, capture_output=True, text=True, timeout=120)
    got = sorted(p.name for p in (d / "s").glob("scan-*.jpg"))
    job = Scanner.jobs[-1] if Scanner.jobs else ""
    ok = r.returncode == 0 and got == want_files and all(re.search(w, job) for w in want_job)
    print(("ok   " if ok else "FAIL ") + name)
    if not ok: fail.append((name, r.returncode, got, (r.stderr or r.stdout)[-200:]))
run("feeder, all pages", ["--scan-source", "feeder", "--scan-only"], ["scan-0001.jpg", "scan-0002.jpg", "scan-0003.jpg"], [r"Feeder"])
run("feeder, stop after 2", ["--scan-source", "feeder", "--scan-pages", "2", "--scan-only"], ["scan-0001.jpg", "scan-0002.jpg"])
run("platen gives one page", ["--scan-source", "platen", "--scan-only"], ["scan-0001.jpg"], [r"Platen"])
run("dpi and mode in the request", ["--scan-dpi", "150", "--scan-mode", "gray", "--scan-only"], ["scan-0001.jpg"], [r"150", r"Gray"])
# without --scan-only the pages are processed into a PDF
d = pathlib.Path(tempfile.mkdtemp())
r = subprocess.run([exe, "--scan-escl", target, "--scan-dir", str(d / "s"), "--scan-source", "feeder", "--pdf", str(d / "o.pdf")], capture_output=True, text=True, timeout=120)
ok = r.returncode == 0 and (d / "o.pdf").exists() and (d / "o.pdf").stat().st_size > 500
print(("ok   " if ok else "FAIL ") + "scan and process into a PDF")
if not ok: fail.append(("pdf", r.returncode, (r.stderr or r.stdout)[-200:]))
srv.shutdown()
print("all passed" if not fail else f"{len(fail)} failed")
for f in fail: print("  ", f)
sys.exit(1 if fail else 0)
