"""Dialog scene (export): needs a real file dialog, see .github/workflows/gui-clips.yml."""
DURATION = 101.0
FILES = ["dublu-stramb.png", "alba.png", "curbat.png"]
WARMUP = 14

import pathlib
O = pathlib.Path("/tmp/o")        # short link to the export folder

def name(d, path):
    d.type_text(str(path), 0.02); d.key("Return")

def pick_folder(d):
    d.click(339, 112); d.click(1170, 832)

def act(tl, d, sc):
    out = O
    tl.at(0.5); d.click(20, 12); tl.at(2); d.click(56, 150)          # File > Export PDF…
    tl.at(6); d.shot("/tmp/gui/c09_dialog.png"); name(d, out / "livre.pdf")
    tl.at(18); d.shot("/tmp/gui/c09_s2.png")
    tl.at(26); d.click(20, 12); tl.at(28); d.click(56, 172)          # Export images…
    tl.at(31); d.shot("/tmp/gui/c09_s3.png")
    tl.at(33); pick_folder(d)
    tl.at(45); d.shot("/tmp/gui/c09_s4.png")
    tl.at(51); d.click(20, 12); tl.at(53); d.click(56, 117)          # Save project…
    tl.at(56); name(d, out / "proiect.lgr")
    tl.at(62); d.click(20, 12); tl.at(64); d.click(56, 227)          # Save project to folder…
    tl.at(68); pick_folder(d)
    tl.at(74); d.shot("/tmp/gui/c09_s5.png")
    tl.at(80); d.click(20, 12); tl.at(82); d.click(56, 94)           # Open project…
    tl.at(86); d.key("l", ["Control_L"]); name(d, out / "proiect.lgr")
