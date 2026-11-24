"""Dialog scene (export): needs a real file dialog, see .github/workflows/gui-clips.yml."""
DURATION = 101.0
FILES = ["dublu-stramb.png", "alba.png", "curbat.png"]
WARMUP = 14

def name(d, path):
    d.type_text(str(path), 0.04); d.key("Return")

def act(tl, d, sc):
    out = sc.parent / "export"; out.mkdir(exist_ok=True)
    tl.at(0.5); d.click(20, 12); tl.at(2); d.click(56, 150)          # File > Export PDF…
    tl.at(6); d.shot("/tmp/gui/c09_dialog.png"); name(d, out / "livre.pdf")
    tl.at(26); d.click(20, 12); tl.at(28); d.click(56, 172)          # Export images…
    tl.at(33); d.key("l", ["Control_L"]); name(d, out)
    tl.at(51); d.click(20, 12); tl.at(53); d.click(56, 117)          # Save project…
    tl.at(56); name(d, out / "proiect.lgr")
    tl.at(62); d.click(20, 12); tl.at(64); d.click(56, 227)          # Save project to folder…
    tl.at(68); d.key("l", ["Control_L"]); name(d, out)
    tl.at(80); d.click(20, 12); tl.at(82); d.click(56, 94)           # Open project…
    tl.at(86); d.key("l", ["Control_L"]); name(d, out / "proiect.lgr")
