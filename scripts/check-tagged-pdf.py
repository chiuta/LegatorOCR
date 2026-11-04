#!/usr/bin/env python3
"""Self-check of a tagged PDF written by `legatorocr --tagged-pdf` (needs pikepdf).

    check-tagged-pdf.py OUT.pdf [--passages PASSAGES.jsonl]

This is a consistency check of the structure we write, NOT a PDF/UA validator (veraPDF is the
reference and was not available). It confirms:
  1. catalog: /MarkInfo /Marked true, /Lang, /ViewerPreferences /DisplayDocTitle true,
     /StructTreeRoot, and a non-empty document /Title (Info);
  2. every page has /Tabs /S and /StructParents;
  3. every marked-content id (MCID) in a page's content stream has exactly one structure
     element, every non-Artifact marked-content sequence has an MCID, BDC/EMC are balanced;
  4. every structure element is reachable from the root and is listed in the /ParentTree,
     and /ParentTree[page][mcid] is the element that owns that MCID;
  5. reading order: the tree order equals the content-stream order, page by page;
  6. with --passages (the `--passages-jsonl` file of the same run): per page the words of the
     structure elements, in tree order, equal the words of the OCR paragraphs, and every OCR
     paragraph boundary is also an element boundary (headings may split a paragraph further).
Exit status 0 = all checks passed.
"""
import json
import re
import sys

import pikepdf


def die(msg):
    print("FAIL:", msg)
    sys.exit(1)


def cmap_of(font):
    """CID -> text from the font's /ToUnicode (bfchar / bfrange as the writer emits them)."""
    tu = font.get("/ToUnicode")
    if tu is None:
        die("font has no /ToUnicode")
    data = tu.read_bytes().decode("latin-1")
    m = {}
    for blk in re.findall(r"beginbfchar(.*?)endbfchar", data, re.S):
        for a, b in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>", blk):
            m[int(a, 16)] = bytes.fromhex(b).decode("utf-16-be")
    for blk in re.findall(r"beginbfrange(.*?)endbfrange", data, re.S):
        for a, b, c in re.findall(r"<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>\s*<([0-9A-Fa-f]+)>", blk):
            lo, hi, dst = int(a, 16), int(b, 16), int(c, 16)
            for k in range(lo, hi + 1):
                m[k] = chr(dst + k - lo)
    return m


def page_mcids(page, cmap):
    """[(tag, mcid, text)] in content-stream order; checks nesting."""
    out, stack = [], []
    for operands, op in pikepdf.parse_content_stream(page):
        o = str(op)
        if o == "BMC":
            stack.append(None)
        elif o == "BDC":
            tag = str(operands[0])
            props = operands[1]
            if tag == "/Artifact":
                stack.append(None)
                continue
            if not isinstance(props, pikepdf.Dictionary) or "/MCID" not in props:
                die(f"BDC {tag} without MCID")
            out.append([tag, int(props["/MCID"]), ""])
            stack.append(len(out) - 1)
        elif o == "EMC":
            if not stack:
                die("EMC without BDC")
            stack.pop()
        elif o in ("Tj", "TJ"):
            idx = next((s for s in reversed(stack) if s is not None), None)
            if idx is None:
                die("text outside any tagged sequence and outside an Artifact")
            raw = operands[0]
            parts = [raw] if o == "Tj" else [x for x in operands[0] if isinstance(x, pikepdf.String)]
            for p in parts:
                b = bytes(p)
                for i in range(0, len(b) - 1, 2):
                    out[idx][2] += cmap.get((b[i] << 8) | b[i + 1], "?")
    if stack:
        die("unbalanced BDC/EMC")
    return out


def main():
    args = sys.argv[1:]
    if not args:
        die("usage: check-tagged-pdf.py OUT.pdf [--passages FILE.jsonl]")
    path, passages = args[0], None
    if "--passages" in args:
        passages = args[args.index("--passages") + 1]
    pdf = pikepdf.open(path)
    root = pdf.Root

    # 1. catalog
    if not (root.get("/MarkInfo") is not None and bool(root.MarkInfo.get("/Marked"))):
        die("/MarkInfo /Marked true missing")
    if "/Lang" not in root or not str(root.Lang):
        die("/Lang missing")
    if not (root.get("/ViewerPreferences") is not None and bool(root.ViewerPreferences.get("/DisplayDocTitle"))):
        die("/ViewerPreferences /DisplayDocTitle true missing")
    title = pdf.docinfo.get("/Title")
    if title is None or not str(title).strip():
        die("document /Title missing")
    if "/StructTreeRoot" not in root:
        die("/StructTreeRoot missing")
    st = root.StructTreeRoot
    print(f"lang={str(root.Lang)} title={str(title)!r}")

    # 2. pages
    font = None
    for pg in pdf.pages:
        if str(pg.get("/Tabs")) != "/S":
            die("page without /Tabs /S")
        if "/StructParents" not in pg:
            die("page without /StructParents")
        if font is None:
            font = pg.Resources.Font.F1.DescendantFonts[0] if False else pg.Resources.Font.F1
    cmap = cmap_of(font) if font is not None else {}
    pnum = {pg.objgen: i for i, pg in enumerate(pdf.pages)}

    # 3. content-stream marked content
    content = [page_mcids(pg, cmap) for pg in pdf.pages]
    for i, seq in enumerate(content):
        ids = [m[1] for m in seq]
        if ids != list(range(len(ids))):
            die(f"page {i + 1}: MCIDs are not 0..n-1 in stream order: {ids}")

    # 4. tree walk from the root
    elems = []  # (objgen, tag, page index, mcid) in document order

    def walk(node, depth=0):
        if depth > 50:
            die("structure tree too deep / cyclic")
        kids = node.get("/K")
        if kids is None:
            return
        if not isinstance(kids, pikepdf.Array):
            kids = [kids]
        for k in kids:
            if isinstance(k, int) or (isinstance(k, pikepdf.Object) and k._type_code == pikepdf.ObjectType.integer):
                pg = node.get("/Pg")
                if pg is None:
                    die("element with an MCID but no /Pg")
                elems.append((node.objgen, str(node.get("/S")), pnum[pg.objgen], int(k)))
            elif isinstance(k, pikepdf.Dictionary):
                if str(k.get("/Type")) == "/MCR":
                    die("MCR dictionaries are not written by this program")
                par = k.get("/P")
                if par is None or par.objgen != node.objgen:
                    die(f"element {k.objgen} /P does not point to its parent")
                walk(k, depth + 1)

    walk(st)
    if not elems:
        die("structure tree holds no marked content")
    seen = set()
    for g, tag, p, m in elems:
        if (p, m) in seen:
            die(f"MCID {m} on page {p + 1} owned by two elements")
        seen.add((p, m))
    stream_ids = {(i, m[1]) for i, seq in enumerate(content) for m in seq}
    if seen != stream_ids:
        die(f"MCIDs differ: only in tree {sorted(seen - stream_ids)[:5]}, only in streams {sorted(stream_ids - seen)[:5]}")
    # tags agree
    tag_of = {(p, m): t for _, t, p, m in elems}
    for i, seq in enumerate(content):
        for tag, mc, _ in seq:
            if tag_of[(i, mc)] != tag:
                die(f"page {i + 1} MCID {mc}: content tag {tag} but element {tag_of[(i, mc)]}")

    # ParentTree
    nums = st.ParentTree.Nums
    ptree = {int(nums[j]): nums[j + 1] for j in range(0, len(nums), 2)}
    for i, pg in enumerate(pdf.pages):
        arr = ptree.get(int(pg.StructParents))
        if arr is None:
            die(f"page {i + 1}: /StructParents not in /ParentTree")
        for mc in range(len(content[i])):
            owner = arr[mc]
            if (owner.objgen, str(owner.S), i, mc) not in elems:
                die(f"page {i + 1}: /ParentTree[{mc}] is not the element owning that MCID")
    listed = {a.objgen for arr in ptree.values() for a in arr}
    if listed != {e[0] for e in elems}:
        die("an element is missing from /ParentTree (or vice versa)")

    # 5. reading order = content order
    order_tree = [(p, m) for _, _, p, m in elems]
    order_stream = [(i, m[1]) for i, seq in enumerate(content) for m in seq]
    if order_tree != order_stream:
        die("tree order differs from content-stream order")
    n_by_tag = {}
    for _, t, _, _ in elems:
        n_by_tag[t] = n_by_tag.get(t, 0) + 1
    print(f"{len(pdf.pages)} page(s), {len(elems)} elements {n_by_tag}")

    # 6. against the OCR paragraphs
    if passages:
        per_page = {}
        for line in open(passages, encoding="utf-8"):
            if line.strip():
                j = json.loads(line)
                per_page.setdefault(j["page"], []).append(j["text"].split())
        for i, seq in enumerate(content):
            got = [t.split() for _, _, t in seq]
            want = per_page.get(i + 1, [])
            if sum(got, []) != sum(want, []):
                die(f"page {i + 1}: words in tree order differ from the OCR paragraphs")
            cuts = lambda ps: {sum(len(x) for x in ps[:k]) for k in range(len(ps) + 1)}
            if not cuts(want) <= cuts(got):
                die(f"page {i + 1}: an OCR paragraph boundary is not an element boundary")
            print(f"page {i + 1}: {len(want)} OCR paragraph(s) -> {len(got)} element(s), same words in same order")
    print("OK (self-consistency only; not a PDF/UA validation)")


main()
