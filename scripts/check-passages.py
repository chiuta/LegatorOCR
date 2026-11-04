#!/usr/bin/env python3
"""Validate the file written by `legatorocr --passages-jsonl` (stdlib only).

  python3 scripts/check-passages.py PASSAGES.jsonl [--txt OUT.txt] [--images DIR]

Always checked, for every line: it is a JSON object with exactly the documented keys and types
(see "Passage export" in README.md); schema value; page >= 1; paragraph_index counts 0,1,2.. per
page; bbox_px is 4 integers with 0 <= x0 < x1 <= page_width_px and 0 <= y0 < y1 <= page_height_px;
bbox_frac equals bbox_px / page size (tolerance 2e-6) and lies in [0,1]; mean_confidence is null or
in [0,100]; page_image_sha256 is 64 lower-case hex digits and the same for all paragraphs of a page;
page size is the same for all paragraphs of a page; passage_id is the first 16 hex digits of
SHA-256("<page_image_sha256>:x0,y0,x1,y1") and unique in the file; text is not blank; cite is
"<title> p.<page_label or page>"; page_image_basis is "file" or "png-encoding" and
page_image_file is a string exactly when the basis is "file".
--images DIR: for basis "file" the SHA-256 of DIR/page_image_file must equal page_image_sha256.
--txt FILE: the passages of each page (joined by a space) must equal that page of the --txt output
(pages split at form feed) after collapsing white space; prints the match percentage (character
level, difflib ratio, over all pages) and the number of pages that match exactly.
Exit status 0 when everything checked passes, 1 otherwise.
"""
import difflib, hashlib, json, os, re, sys

KEYS = {
    "schema": str, "passage_id": str, "source_file": str, "page": int, "page_label": (str, type(None)),
    "paragraph_index": int, "text": str, "bbox_px": list, "bbox_frac": list, "page_width_px": int,
    "page_height_px": int, "language": str, "mean_confidence": (int, float, type(None)),
    "page_image_sha256": str, "page_image_file": (str, type(None)), "page_image_basis": str, "cite": str,
}
HEX64 = re.compile(r"^[0-9a-f]{64}$")


def norm(s):
    return " ".join(s.split())


def main(argv):
    args = [a for a in argv if not a.startswith("--")]
    opts = {}
    it = iter(argv)
    for a in it:
        if a in ("--txt", "--images"):
            opts[a] = next(it)
    path = argv[0]
    errs = []

    def bad(n, msg):
        if len(errs) < 40:
            errs.append(f"line {n}: {msg}")

    rows = []
    for n, line in enumerate(open(path, encoding="utf-8"), 1):
        if not line.endswith("\n"):
            bad(n, "line does not end with a newline")
        try:
            o = json.loads(line)
        except ValueError as e:
            bad(n, f"not JSON: {e}")
            continue
        if not isinstance(o, dict):
            bad(n, "not an object")
            continue
        if set(o) != set(KEYS):
            bad(n, f"keys differ: missing {sorted(set(KEYS) - set(o))}, extra {sorted(set(o) - set(KEYS))}")
            continue
        ok = True
        for k, t in KEYS.items():
            v = o[k]
            if not isinstance(v, t) or (isinstance(v, bool)):
                bad(n, f"{k}: wrong type {type(v).__name__}")
                ok = False
        if not ok:
            continue
        rows.append((n, o))
        if o["schema"] != "legatorocr.passage/1":
            bad(n, "schema value")
        if o["page"] < 1:
            bad(n, "page < 1")
        if not o["text"].strip():
            bad(n, "blank text")
        W, H = o["page_width_px"], o["page_height_px"]
        if W < 1 or H < 1:
            bad(n, "page size < 1")
        px, fr = o["bbox_px"], o["bbox_frac"]
        if len(px) != 4 or not all(type(v) is int for v in px):
            bad(n, "bbox_px must be 4 integers")
        elif not (0 <= px[0] < px[2] <= W and 0 <= px[1] < px[3] <= H):
            bad(n, f"bbox_px {px} not inside the {W}x{H} page")
        elif len(fr) != 4 or not all(isinstance(v, (int, float)) for v in fr):
            bad(n, "bbox_frac must be 4 numbers")
        else:
            exp = [px[0] / W, px[1] / H, px[2] / W, px[3] / H]
            if any(abs(a - b) > 2e-6 for a, b in zip(fr, exp)) or not all(0 <= v <= 1 for v in fr):
                bad(n, f"bbox_frac {fr} does not match bbox_px/page size {exp}")
            if o["passage_id"] != hashlib.sha256(f"{o['page_image_sha256']}:{px[0]},{px[1]},{px[2]},{px[3]}".encode()).hexdigest()[:16]:
                bad(n, "passage_id is not the documented hash")
        mc = o["mean_confidence"]
        if mc is not None and not 0 <= mc <= 100:
            bad(n, "mean_confidence outside 0..100")
        if not HEX64.match(o["page_image_sha256"]):
            bad(n, "page_image_sha256 not 64 lower-case hex digits")
        b = o["page_image_basis"]
        if b not in ("file", "png-encoding"):
            bad(n, "page_image_basis value")
        elif (b == "file") != isinstance(o["page_image_file"], str):
            bad(n, "page_image_file must be a string exactly when basis is 'file'")
        lab = o["page_label"] if o["page_label"] is not None else str(o["page"])
        if not o["cite"].endswith(f" p.{lab}") or len(o["cite"]) <= len(f" p.{lab}"):
            bad(n, f"cite should end with ' p.{lab}' and have a title before it: {o['cite']!r}")

    ids = [o["passage_id"] for _, o in rows]
    if len(ids) != len(set(ids)):
        # the same box on the same page image can only repeat if the file lists a page twice
        errs.append("passage_id values are not unique")
    pages = {}
    for n, o in rows:
        pages.setdefault((o["source_file"], o["page"]), []).append((n, o))
    for (src, pg), lst in pages.items():
        for k in ("page_image_sha256", "page_width_px", "page_height_px", "page_label", "language", "page_image_file", "page_image_basis"):
            if len({json.dumps(o[k]) for _, o in lst}) != 1:
                errs.append(f"page {pg} of {src}: {k} differs between its paragraphs")
        if [o["paragraph_index"] for _, o in lst] != list(range(len(lst))):
            errs.append(f"page {pg} of {src}: paragraph_index is not 0,1,2,...")
    if len({o["page"] for _, o in rows}) != len({k[1] for k in pages}):
        pass
    if not rows:
        errs.append("no passages in the file")

    if "--images" in opts:
        seen = 0
        for (src, pg), lst in pages.items():
            o = lst[0][1]
            if o["page_image_basis"] == "file":
                f = os.path.join(opts["--images"], o["page_image_file"])
                h = hashlib.sha256(open(f, "rb").read()).hexdigest()
                seen += 1
                if h != o["page_image_sha256"]:
                    errs.append(f"{f}: SHA-256 {h} differs from page_image_sha256")
        print(f"image hashes checked against files: {seen} page(s)")

    if "--txt" in opts:
        txt = open(opts["--txt"], encoding="utf-8").read().split("\x0c")
        by_page = {}
        for (src, pg), lst in sorted(pages.items(), key=lambda kv: kv[0][1]):
            by_page[pg] = norm(" ".join(o["text"] for _, o in lst))
        tot_a = tot_m = exact = 0
        for i, t in enumerate(txt, 1):
            a, b = norm(t), by_page.get(i, "")
            sm = difflib.SequenceMatcher(None, a, b, autojunk=False)
            m = sum(x.size for x in sm.get_matching_blocks())
            tot_a += max(len(a), len(b))
            tot_m += m
            exact += a == b
            print(f"page {i}: txt {len(a)} chars, passages {len(b)} chars, {'identical' if a == b else f'{100 * m / max(len(a), len(b), 1):.2f}% match'}")
        print(f"text match over all pages: {100 * tot_m / max(tot_a, 1):.2f}% of characters; {exact}/{len(txt)} pages identical")
        if exact != len(txt):
            errs.append("passage text differs from --txt on some page")

    if errs:
        print("FAIL")
        for e in errs:
            print(" ", e)
        return 1
    print(f"OK: {len(rows)} passages on {len(pages)} page(s)")
    return 0


if __name__ == "__main__":
    if len(sys.argv) < 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1:]))
