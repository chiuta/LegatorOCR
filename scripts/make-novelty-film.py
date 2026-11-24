#!/usr/bin/env python3
"""Films the new command-line options of pre.74 as a terminal recording: the commands are REALLY run
on generated pages and photos, and their output is typed into drawn terminal frames. Writes
docs/video/noutati.mp4, noutati.<xx>.vtt (8 languages) and noutati.jpg.
Usage: scripts/make-novelty-film.py [path/to/legatorocr]   (needs Pillow and ffmpeg)"""
import subprocess, sys, os, tempfile, pathlib, datetime, shutil
from PIL import Image, ImageDraw, ImageFont

root = pathlib.Path(__file__).resolve().parent.parent
exe = sys.argv[1] if len(sys.argv) > 1 else str(root / "target/release/legatorocr")
work = pathlib.Path(tempfile.mkdtemp())
(work / "pagini").mkdir(); (work / "fotografii").mkdir()
DJ = "/usr/share/fonts/truetype/dejavu/"
f, fb = ImageFont.truetype(DJ + "DejaVuSerif.ttf", 34), ImageFont.truetype(DJ + "DejaVuSerif-Bold.ttf", 50)

# --- invented pages (A4 at 200 dpi) and phone photos with EXIF times (10 s rhythm, two deleted shots)
pages = [("Capitolul întâi", ["Zorgblat a plecat spre munte în zori.", "Pe cărare l-a întâlnit pe moș Ilie,", "care ducea oile la stână, ca în fiecare an.", "Comanda se numește ABC-123-45 în registru."]),
         ("Capitolul al doilea", ["Seara, la gura sobei, bătrânul a povestit", "cum s-au așezat satele între ape.", "Nimeni nu mai știa vechiul drum,", "dar toți îl purtau în nume și în cântec."])]
for n, (t, ls) in enumerate(pages, 1):
    im = Image.new("L", (1654, 2339), 240); d = ImageDraw.Draw(im)
    d.text((160, 200), t, font=fb, fill=30)
    for i, l in enumerate(ls * 3): d.text((160, 360 + i * 70), l, font=f, fill=40)
    im.save(work / "pagini" / f"pagina-{n}.png", dpi=(200, 200))
t0, t = datetime.datetime(2026, 11, 22, 10, 15, 0), 0
times = []
for g in [0, 10, 10, 10, 20, 10, 10, 10, 10, 20, 10, 10, 10, 10, 10, 10]:
    t += g; times.append(t)
for i, s in enumerate(times):
    im = Image.new("RGB", (1200, 1600), (225, 222, 214)); d = ImageDraw.Draw(im)
    d.text((100, 120), f"Pagina {i+1}", font=fb, fill=(30, 30, 30))
    for k in range(18): d.text((100, 260 + k * 70), "Rând de text %d, pagina %d, cu ceva cifre 20%d." % (k, i + 1, k + i), font=f, fill=(45, 45, 45))
    ex = Image.Exif(); ex[0x8769] = {0x9003: (t0 + datetime.timedelta(seconds=s)).strftime("%Y:%m:%d %H:%M:%S")}
    im.save(work / "fotografii" / f"IMG_{i+1:04d}.jpg", exif=ex, quality=85)

def run(args, show):
    r = subprocess.run([exe] + args, cwd=work, capture_output=True, text=True)
    lines = [l for l in (r.stderr + r.stdout).splitlines() if not l.startswith((" ", "Stack"))]
    return [l for l in lines if show(l)]

c1 = "legatorocr pagini/*.png --ocr -l ron --markdown carte.md --format png --cbz carte.cbz -o iesire"
o1 = run(["pagini/pagina-1.png", "pagini/pagina-2.png", "--ocr", "-l", "ron", "--markdown", "carte.md", "--format", "png", "--cbz", "carte.cbz", "-o", "iesire"], lambda l: l.startswith(("wrote", "OCR engine")))
md = (work / "carte.md").read_text().splitlines()[:12]
c2, o2 = "head -n 12 carte.md", md
c3 = "legatorocr --burst fotografii/*.jpg"
o3 = run(["--burst"] + [f"fotografii/IMG_{i+1:04d}.jpg" for i in range(len(times))] + ["-o", "b"], lambda l: l.startswith("--burst: ") and "repeated" not in l)
assert o1 and o3 and any("page" in l for l in md), (o1, o3, md)

W, H, FPS = 1280, 720, 15
mono = ImageFont.truetype(DJ + "DejaVuSansMono.ttf", 21)
monob = ImageFont.truetype(DJ + "DejaVuSansMono-Bold.ttf", 21)
def wrap(s, n=82):
    out = []
    while len(s) > n: out.append(s[:n]); s = "  " + s[n:]
    return out + [s]
scenes = [(c1, o1), (c2, o2), (c3, o3)]
SCENE_S = 13.0
frames_dir = work / "fr"; frames_dir.mkdir()
k = 0
for c, o in scenes:
    for fi in range(int(SCENE_S * FPS)):
        tm = fi / FPS
        typed = c[: int(max(0, tm - 0.4) * 38)]
        shown = 0 if tm < 3.2 else min(len(o), int((tm - 3.2) / 0.55) + 1)
        im = Image.new("RGB", (W, H), (10, 16, 22)); d = ImageDraw.Draw(im)
        d.rectangle([0, 0, W, 34], fill=(26, 36, 46)); d.text((14, 6), "LegatorOCR 0.2.0-pre.74 — terminal (real output)", font=ImageFont.truetype(DJ + "DejaVuSans.ttf", 17), fill=(150, 170, 190))
        y = 52
        cursor = "▌" if int(tm * 2) % 2 == 0 and shown == 0 else ""
        for i, ln in enumerate(wrap("$ " + typed + cursor)):
            d.text((20, y), ln, font=monob, fill=(120, 220, 140)); y += 29
        y += 8
        for ln in o[:shown]:
            for w2 in wrap(ln):
                d.text((20, y), w2, font=mono, fill=(215, 225, 235)); y += 29
        im.save(frames_dir / f"f{k:05d}.png"); k += 1
vid = root / "docs/video"
subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", str(FPS), "-i", str(frames_dir / "f%05d.png"), "-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-pix_fmt", "yuv420p", "-movflags", "+faststart", str(vid / "noutati.mp4")], check=True)
shutil.copy(frames_dir / f"f{int(SCENE_S*FPS*2-1):05d}.png", work / "poster.png")
Image.open(work / "poster.png").convert("RGB").save(vid / "noutati.jpg", quality=80)

T = {  # two cues per scene, in the 8 languages
"ro": ["Textul recunoscut ca Markdown și paginile ca arhivă CBZ, într-o singură trecere.", "Titlurile devin ##, fiecare pagină are marcajul ei; textul nu devine structură Markdown.", "Cu --burst: ritmul real de 10 s și cele 2 cadre șterse, deduse din golurile de 20 s.", "Timerul sugerat urcă la 13 s, pentru că peste 10 % din sloturi s-au irosit."],
"en": ["The recognised text as Markdown and the pages as a CBZ archive, in one pass.", "Headings become ##, every page has its mark; text never turns into Markdown structure.", "With --burst: the real 10 s rhythm and the 2 deleted shots, found from the 20 s gaps.", "The suggested timer rises to 13 s because over 10 % of the slots were wasted."],
"fr": ["Le texte reconnu en Markdown et les pages en archive CBZ, en une seule passe.", "Les titres deviennent ##, chaque page a son repère ; le texte ne devient jamais structure Markdown.", "Avec --burst : le vrai rythme de 10 s et les 2 prises supprimées, déduites des écarts de 20 s.", "Le minuteur suggéré monte à 13 s, car plus de 10 % des créneaux ont été perdus."],
"it": ["Il testo riconosciuto in Markdown e le pagine in archivio CBZ, in un solo passaggio.", "I titoli diventano ##, ogni pagina ha il suo segno; il testo non diventa mai struttura Markdown.", "Con --burst: il vero ritmo di 10 s e i 2 scatti cancellati, dedotti dagli intervalli di 20 s.", "Il timer suggerito sale a 13 s perché oltre il 10 % degli slot è andato perso."],
"es": ["El texto reconocido como Markdown y las páginas como archivo CBZ, en una sola pasada.", "Los títulos pasan a ##, cada página lleva su marca; el texto nunca se convierte en estructura Markdown.", "Con --burst: el ritmo real de 10 s y las 2 tomas borradas, deducidas de los huecos de 20 s.", "El temporizador sugerido sube a 13 s porque más del 10 % de los turnos se perdió."],
"pt": ["O texto reconhecido como Markdown e as páginas como arquivo CBZ, numa só passagem.", "Os títulos viram ##, cada página tem a sua marca; o texto nunca se torna estrutura Markdown.", "Com --burst: o ritmo real de 10 s e os 2 disparos apagados, deduzidos dos intervalos de 20 s.", "O temporizador sugerido sobe para 13 s porque mais de 10 % dos intervalos se perdeu."],
"de": ["Der erkannte Text als Markdown und die Seiten als CBZ-Archiv, in einem Durchgang.", "Überschriften werden zu ##, jede Seite hat ihre Marke; Text wird nie zu Markdown-Struktur.", "Mit --burst: der echte 10-s-Rhythmus und die 2 gelöschten Aufnahmen, aus den 20-s-Lücken abgeleitet.", "Der vorgeschlagene Timer steigt auf 13 s, weil über 10 % der Zeitfenster verloren gingen."],
"hu": ["A felismert szöveg Markdownként, az oldalak CBZ-archívumként, egyetlen menetben.", "A címsorok ##-sá válnak, minden oldalnak megvan a jelölése; a szöveg sosem lesz Markdown-szerkezet.", "A --burst-tel: a valódi 10 mp-es ritmus és a 2 törölt felvétel, a 20 mp-es hézagokból kikövetkeztetve.", "A javasolt időzítő 13 mp-re nő, mert a helyek több mint 10 %-a kárba veszett."]}
def ts(s):
    return "%02d:%02d:%06.3f" % (s // 3600, (s % 3600) // 60, s % 60)
for lang, tx in T.items():
    cues = []
    for sc in range(3):
        base = sc * SCENE_S
        if sc == 0:
            cues.append((base + 0.5, base + 12.5, tx[0]))
        elif sc == 1:
            cues.append((base + 0.5, base + 12.5, tx[1]))
        else:
            cues.append((base + 0.5, base + 6.5, tx[2]))
            cues.append((base + 7.0, base + 12.6, tx[3]))
    (vid / f"noutati.{lang}.vtt").write_text("WEBVTT\n\n" + "\n".join(f"{ts(s)} --> {ts(e)}\n{t}\n" for s, e, t in cues))
shutil.rmtree(work)
print("wrote docs/video/noutati.mp4 and 8 subtitle files")
