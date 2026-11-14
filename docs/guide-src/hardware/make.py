#!/usr/bin/env python3
"""Writes docs/guide-src/hardware[.xx].html from content.py. Run: python3 docs/guide-src/hardware/make.py
(then `node docs/guide-src/build.js` makes the PDFs)."""
import html, pathlib, sys
sys.path.insert(0, str(pathlib.Path(__file__).parent))
from content import T, LANGS

OUT = pathlib.Path(__file__).resolve().parent.parent
e = html.escape

CSS = ('@page{size:A4;margin:18mm 16mm}body{font-family:"DejaVu Sans","Liberation Sans",sans-serif;font-size:10pt;'
       'line-height:1.45;color:#111}h1{font-size:22pt;margin:0 0 2mm}h2+p{break-before:avoid}h2{font-size:14pt;margin:8mm 0 3mm;border-bottom:1px solid #bbb;'
       'padding-bottom:1mm;break-after:avoid}h3{font-size:11pt;margin:5mm 0 2mm;break-after:avoid}.sub{font-size:12pt;color:#444}'
       '.by{font-size:9pt;color:#555;margin:3mm 0 6mm}img{max-width:100%;height:auto;display:block;margin:3mm auto;'
       'border:1px solid #ccc}figure{margin:4mm 0;break-inside:avoid}figcaption{font-size:8.5pt;color:#444;text-align:center}'
       'table{border-collapse:collapse;width:100%;margin:3mm 0;break-inside:avoid}th,td{border:1px solid #bbb;padding:2mm 2.5mm;'
       'vertical-align:top;font-size:9pt;text-align:left}th{background:#eee}td.n{text-align:right;white-space:nowrap}'
       'tr.tot td{font-weight:bold;background:#f4f4f4}.attn{border-left:4px solid #d08a00;background:#fff6e0;padding:2mm 3mm;'
       'margin:3mm 0;break-inside:avoid}.photos{display:flex;gap:3mm;align-items:flex-start}.photos figure{flex:1;margin:0}'
       '.photos img{margin:0 auto}.step{display:flex;gap:3mm;margin:1.5mm 0;break-inside:avoid}.step b{flex:0 0 7mm;height:7mm;'
       'line-height:7mm;border-radius:50%;background:#1d4f91;color:#fff;text-align:center;font-size:9pt}.step span{flex:1}'
       'ul{margin:2mm 0 2mm 5mm;padding:0}li{margin:1mm 0}.toc li{list-style:none}')

def steps(items):
    return ''.join(f'<div class="step"><b>{i}</b><span>{e(t)}</span></div>' for i, t in enumerate(items, 1))

def table(head, rows, total=False):
    h = ''.join(f'<th>{e(c)}</th>' for c in head)
    out = []
    for i, r in enumerate(rows):
        last = total and i == len(rows) - 1
        cells = ''.join(f'<td class="{"n" if total and j == len(r) - 1 else ""}">{e(c)}</td>' for j, c in enumerate(r))
        out.append(f'<tr class="{"tot" if last else ""}">{cells}</tr>')
    return f'<table><thead><tr>{h}</tr></thead><tbody>{"".join(out)}</tbody></table>'

def page(L):
    t = T[L]
    heads = [t[k] for k in ("h_intro", "h_cmp", "h_cost", "h_rig", "h_shoot", "h_sw", "h_tips", "h_law", "h_src")]
    toc = ''.join(f'<li>{e(h)}</li>' for h in heads)
    p = []
    p.append(f'<!doctype html><html lang="{L}"><head><meta charset="utf-8"><title>{e(t["title"])} ({{{{VERSION}}}})</title><style>{CSS}</style></head><body>')
    p.append(f'<h1>{e(t["title"])}</h1><div class="sub">{e(t["sub"])} – LegatorOCR {{{{VERSION}}}}</div><div class="by">{e(t["by"])}</div>')
    p.append(f'<h3>{e(t["toc"])}</h3><ul class="toc">{toc}</ul>')
    p.append(f'<h2>{e(t["h_intro"])}</h2><p>{e(t["intro1"])}</p><p>{e(t["intro2"])}</p>')
    p.append(f'<figure><img src="img/hardware/diagram.png" alt=""><figcaption>{e(t["fig_diag"])}</figcaption></figure>')
    p.append(f'<h2>{e(t["h_cmp"])}</h2><p>{e(t["cmp_p"])}</p>{table(t["cmp_head"], t["cmp_rows"])}<p>{e(t["cmp_note"])}</p>')
    p.append(f'<h2>{e(t["h_cost"])}</h2><p>{e(t["cost_p"])}</p>{table(t["cost_head"], t["cost_rows"], True)}<p>{e(t["cost_note"])}</p>')
    p.append(f'<h2>{e(t["h_rig"])}</h2>{steps(t["rig_steps"])}<div class="attn">{e(t["rig_attn"])}</div>')
    p.append(f'<h3>{e(t["rig_tips_h"])}</h3><ul>{"".join(f"<li>{e(x)}</li>" for x in t["rig_tips"])}</ul>')
    p.append(f'<h2>{e(t["h_shoot"])}</h2>{steps(t["shoot_steps"])}<div class="attn">{e(t["shoot_attn"])}</div>')
    p.append('<div class="photos">' + ''.join(
        f'<figure><img src="img/hardware/{f}" alt=""><figcaption>{e(t[c])}</figcaption></figure>'
        for f, c in (("phone-screen.jpg", "fig_p1"), ("phone-edge.jpg", "fig_p2"), ("operator.jpg", "fig_p3"))) + '</div>')
    p.append(f'<h2>{e(t["h_sw"])}</h2><p>{e(t["sw_p"])}</p>{steps(t["sw_steps"])}')
    p.append(f'<h2>{e(t["h_tips"])}</h2>{table(t["tips_head"], t["tips_rows"])}')
    p.append(f'<h2>{e(t["h_law"])}</h2><ul>{"".join(f"<li>{e(x)}</li>" for x in t["law_items"])}</ul><p><i>{e(t["law_note"])}</i></p>')
    p.append(f'<h2>{e(t["h_src"])}</h2><ul>{"".join(f"<li>{e(x)}</li>" for x in t["src_items"])}</ul>')
    p.append(f'<h3>{e(t["video_h"])}</h3><p>{e(t["video_p"])}</p></body></html>')
    return ''.join(p)

for L in LANGS:
    name = 'hardware.html' if L == 'ro' else f'hardware.{L}.html'
    (OUT / name).write_text(page(L), encoding='utf-8')
    print('wrote', name)
