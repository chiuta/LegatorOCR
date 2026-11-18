#!/usr/bin/env python3
"""Hostile-input smoke test: feeds the command line truncated, corrupted and absurd files and
options, and fails if any run panics, hangs, runs out of memory or dies of a signal. A clean error
message and a non-zero exit code are fine; a crash is not.

  python3 scripts/hostile.py target/release/legatorocr [--work DIR] [--verbose]
"""
import os, random, resource, struct, subprocess, sys, tempfile, zlib, pathlib, argparse

ap = argparse.ArgumentParser()
ap.add_argument("exe")
ap.add_argument("--work")
ap.add_argument("--verbose", action="store_true")
ap.add_argument("--quick", action="store_true", help="skip the cases that are slow on purpose (400 MP image, 3000-page PDF)")
ap.add_argument("--timeout", type=int, default=60)
ap.add_argument("--mem-mb", type=int, default=6000)
a = ap.parse_args()
a.exe = os.path.abspath(a.exe)
W = pathlib.Path(a.work or tempfile.mkdtemp(prefix="lgr-hostile-"))
W.mkdir(parents=True, exist_ok=True)
rnd = random.Random(20261119)


def png(w, h, extra=b"", raw=None):
    def ch(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)
    data = raw if raw is not None else zlib.compress(b"".join(b"\x00" + bytes(min(w, 400)) for _ in range(min(h, 400))))
    return b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", ihdr) + extra + ch(b"IDAT", data) + ch(b"IEND", b"")


def tiff(w, h):
    # little-endian, one strip, uncompressed grey, dimensions as declared
    ent = lambda tag, typ, cnt, val: struct.pack("<HHII", tag, typ, cnt, val)
    ifd = struct.pack("<H", 8) + b"".join([
        ent(256, 4, 1, w), ent(257, 4, 1, h), ent(258, 3, 1, 8), ent(262, 3, 1, 1),
        ent(273, 4, 1, 8 + 2 + 8 * 12 + 4), ent(277, 3, 1, 1), ent(278, 4, 1, h), ent(279, 4, 1, 16)]) + struct.pack("<I", 0)
    return b"II*\x00" + struct.pack("<I", 8) + ifd + bytes(16)


def big_valid_png(w, h):
    """A well-formed grey PNG of w x h pixels that is a few KB on disk: a decompression bomb."""
    def ch(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    z = zlib.compressobj(9)
    row = b"\x00" + bytes(w)
    out = b""
    for _ in range(h):
        out += z.compress(row)
    out += z.flush()
    return b"\x89PNG\r\n\x1a\n" + ch(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 0, 0, 0, 0)) + ch(b"IDAT", out) + ch(b"IEND", b"")


base = png(64, 64)
cases = {}
cases["empty.png"] = b""
cases["one_byte.png"] = b"\x89"
cases["signature_only.png"] = b"\x89PNG\r\n\x1a\n"
cases["trunc_half.png"] = base[: len(base) // 2]
cases["bad_crc.png"] = base[:-8] + b"\xde\xad\xbe\xef" + base[-4:]
cases["zero_width.png"] = png(0, 100)
cases["zero_height.png"] = png(100, 0)
cases["bomb_2e31.png"] = png(2**31 - 1, 2**31 - 1)
cases["bomb_65535.png"] = png(65535, 65535)
cases["bomb_100k_x_100k.png"] = png(100000, 100000)
cases["huge_dpi.png"] = png(64, 64, extra=struct.pack(">I", 9) + b"pHYs" + struct.pack(">IIB", 0xFFFFFFFF, 0xFFFFFFFF, 1) + struct.pack(">I", zlib.crc32(b"pHYs" + struct.pack(">IIB", 0xFFFFFFFF, 0xFFFFFFFF, 1)) & 0xFFFFFFFF))
cases["zero_dpi.png"] = png(64, 64, extra=struct.pack(">I", 9) + b"pHYs" + struct.pack(">IIB", 0, 0, 1) + struct.pack(">I", zlib.crc32(b"pHYs" + struct.pack(">IIB", 0, 0, 1)) & 0xFFFFFFFF))
cases["tiff_ok.tif"] = tiff(4, 4)
cases["tiff_bomb.tif"] = tiff(0x7FFFFFFF, 0x7FFFFFFF)
cases["tiff_trunc.tif"] = tiff(4, 4)[:30]
cases["jpeg_trunc.jpg"] = b"\xff\xd8\xff\xe0\x00\x10JFIF\x00\x01\x01\x00\x00\x01\x00\x01\x00\x00\xff\xdb"
cases["jpeg_bomb.jpg"] = b"\xff\xd8\xff\xc0\x00\x0b\x08\xff\xff\xff\xff\x01\x01\x11\x00\xff\xd9"
cases["pdf_empty.pdf"] = b"%PDF-1.7\n%%EOF\n"
cases["pdf_trunc.pdf"] = b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n"
cases["pdf_loop.pdf"] = (b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [2 0 R] /Count 1 >>\nendobj\n"
                         b"trailer\n<< /Root 1 0 R /Size 3 >>\n%%EOF\n")
cases["pdf_huge_page.pdf"] = (b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n"
                              b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 99999999 99999999] >>\nendobj\ntrailer\n<< /Root 1 0 R /Size 4 >>\n%%EOF\n")
cases["djvu_trunc.djvu"] = b"AT&TFORM\x00\x00\x00\x10DJVUINFO\x00\x00"
cases["gif_trunc.gif"] = b"GIF89a\x01\x00\x01\x00"
cases["bmp_bomb.bmp"] = b"BM" + struct.pack("<IHHI", 60, 0, 0, 54) + struct.pack("<IiiHHIIiiII", 40, 2**31 - 1, 2**31 - 1, 1, 24, 0, 0, 0, 0, 0, 0)
cases["webp_trunc.webp"] = b"RIFF\x10\x00\x00\x00WEBPVP8 "
cases["text_as_png.png"] = b"this is not an image at all\n" * 50
cases["bomb_valid_20000.png"] = big_valid_png(20000, 20000)
cases["bomb_valid_wide.png"] = big_valid_png(60000, 3000)
many = b"".join(b"%d 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] >>\nendobj\n" % (i + 3) for i in range(3000))
cases["pdf_3000_pages.pdf"] = (b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids ["
                               + b" ".join(b"%d 0 R" % (i + 3) for i in range(3000)) + b"] /Count 3000 >>\nendobj\n" + many
                               + b"trailer\n<< /Root 1 0 R /Size 3003 >>\n%%EOF\n")
cases["pdf_count_lie.pdf"] = (b"%PDF-1.7\n1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n2 0 obj\n<< /Type /Pages /Kids [] /Count 2000000000 >>\nendobj\n"
                              b"trailer\n<< /Root 1 0 R /Size 3 >>\n%%EOF\n")
import io, zipfile
zb = io.BytesIO()
with zipfile.ZipFile(zb, "w", zipfile.ZIP_DEFLATED) as zf:
    zf.writestr("a.png", base)
    zf.writestr("../../evil.png", base)
    zf.writestr("/abs/evil.png", base)
    zf.writestr("big.bin", bytes(300_000_000))
cases["zip_bomb.zip"] = zb.getvalue()
cases["cbz_traversal.cbz"] = zb.getvalue()
rnd_blob = bytes(rnd.getrandbits(8) for _ in range(4096))
cases["random.png"] = rnd_blob
cases["random.pdf"] = b"%PDF-1.4\n" + rnd_blob
cases["flipbits.png"] = bytes(b ^ (1 << rnd.randrange(8)) if rnd.random() < 0.02 else b for b in base)

if a.quick:
    for k in ("bomb_valid_20000.png", "bomb_valid_wide.png", "pdf_3000_pages.pdf"):
        cases.pop(k, None)
for n, d in cases.items():
    (W / n).write_bytes(d)
(W / "-dash.png").write_bytes(base)  # a name that looks like an option
(W / "spaced name ; $(id).png").write_bytes(base)
(W / "unicodé-ăîșț.png").write_bytes(base)
(W / "ok.png").write_bytes(base)

runs = []  # (label, args)
for n in cases:
    runs.append((n, [str(W / n), "--pdf", str(W / "out.pdf")]))
    runs.append((n + " +ocr", [str(W / n), "--ocr", "--lang", "ron", "--pdf", str(W / "out.pdf")]))
runs.append(("dash-name", ["--", str(W / "-dash.png"), "--pdf", str(W / "out.pdf")]))
runs.append(("shell-name", [str(W / "spaced name ; $(id).png"), "--pdf", str(W / "out.pdf"), "--pdf", str(W / "o1.pdf")]))
runs.append(("unicode-name", [str(W / "unicodé-ăîșț.png"), "--pdf", str(W / "out.pdf"), "--pdf", str(W / "o2.pdf")]))
runs.append(("missing", [str(W / "nope.png"), "--pdf", str(W / "out.pdf")]))
runs.append(("dir-as-input", [str(W), "--pdf", str(W / "out.pdf")]))
runs.append(("out-is-file", [str(W / "ok.png"), "--out-dir", str(W / "ok.png")]))
runs.append(("pdf-to-unwritable", [str(W / "ok.png"), "--pdf", "/proc/nope/x.pdf"]))
for opt, val in [("--dpi", "0"), ("--dpi", "1"), ("--dpi", "999999999"), ("--dpi", "-5"), ("--dpi", "nan"),
                 ("--output-dpi", "0"), ("--output-dpi", "4294967295"), ("--lang", "../../etc/passwd"),
                 ("--lang", "ron+../x"), ("--lang", "-c"), ("--lang", ""), ("--tessdata", "/proc/self/mem"),
                 ("--engine", "tesseract;id"), ("--mode", "x" * 100000), ("--margins", "nan"), ("--margins", "-1e30"),
                 ("--max-skew", "1e999"), ("--threshold", "9999999999999")]:
    runs.append((f"{opt} {val[:20]}", [str(W / "ok.png"), opt, val, "--pdf", str(W / "out.pdf")]))
# image boxes through project files
box_vals = ["0,0,0,0", "-1,-1,5,5", "nan,0,10,10", "inf,0,10,10", "0,0,1e30,1e30", "1e-30,1e-30,1e-30,1e-30", "5,5", "a,b,c,d",
            "0,0,10,10;" * 400, "0,0,10,10#table", "0,0,99999999999999999999,5", ";;;;", "0,0,10,10;;nan"]
for i, bv in enumerate(box_vals):
    p = W / f"box{i}.lgr"
    p.write_text(f"legatorocr-project 1\nmode = color\n\n[sheet]\npath = {W/'ok.png'}\npage1.page_image_boxes = {bv}\npage1.page_ocr_zones = {bv}\n")
    runs.append((f"project box {bv[:24]}", ["--project", str(p), "--ocr", "--lang", "ron", "--docx", str(W / f"b{i}.docx"), "--pdf", str(W / "out.pdf")]))
for i, txt in enumerate(["", "legatorocr-project 99\n", "legatorocr-project 1\n[sheet]\npath = /etc/shadow\n", "legatorocr-project 1\n[sheet]\npath = ../../../../etc/passwd\n",
                         "legatorocr-project 1\n" + "[sheet]\npath = ok\n" * 20000, "legatorocr-project 1\nmode = " + "x" * 1_000_000 + "\n",
                         "legatorocr-project 1\n[sheet]\n\x00\x00\n", "\xff\xfe\xfd"]):
    p = W / f"proj{i}.lgr"
    p.write_bytes(txt.encode("latin-1"))
    runs.append((f"project hostile #{i}", ["--project", str(p), "--pdf", str(W / "out.pdf")]))

limit = a.mem_mb * 1024 * 1024
def lim():
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))

bad = []
for label, args in runs:
    env = dict(os.environ, RUST_BACKTRACE="0")
    try:
        r = subprocess.run([a.exe] + args, capture_output=True, timeout=a.timeout, preexec_fn=lim, env=env, cwd=W)
    except subprocess.TimeoutExpired:
        bad.append((label, "TIMEOUT >%ds" % a.timeout, ""))
        continue
    err = (r.stderr + r.stdout).decode("utf-8", "replace")
    why = None
    if r.returncode < 0:
        why = f"killed by signal {-r.returncode}"
    elif r.returncode == 101 or "panicked at" in err:
        why = "PANIC"
    elif "memory allocation of" in err or "capacity overflow" in err:
        why = "OUT OF MEMORY"
    if why:
        bad.append((label, why, err.strip().splitlines()[-1][:200] if err.strip() else ""))
    if a.verbose:
        print(f"{'FAIL' if why else 'ok  '} rc={r.returncode} {label}")
print(f"{len(runs)} hostile runs, {len(bad)} crashes")
for b in bad:
    print("  CRASH:", *b, sep=" | ")
sys.exit(1 if bad else 0)
