"""Dialog scene: needs a real file dialog (xdg-desktop-portal-gtk + a window manager): see
.github/workflows/gui-clips.yml."""
DURATION = 81.0
FILES = []
WARMUP = 12

import pathlib
S = pathlib.Path("/tmp/s")        # short link to the scans folder: the file dialog completes every keystroke

def pick(d, path):
    d.key("l", ["Control_L"]); d.type_text(str(path), 0.02); d.key("Return")

def pick_folder(d):
    d.click(339, 112, double=True); import time; time.sleep(1.5); d.click(1170, 832)      # the first folder of the dialog's list, then "Select"

def act(tl, d, sc):
    tl.at(0.5); d.shot("/tmp/gui/c01_welcome.png")
    tl.at(4); d.click(28, 61)                                       # Add
    tl.at(7); d.shot("/tmp/gui/c01_s1.png")
    tl.at(8); d.shot("/tmp/gui/c01_dialog.png"); pick(d, S / "dublu-stramb.png")
    tl.at(15); d.shot("/tmp/gui/c01_s2.png")
    tl.at(25); d.click(20, 12); tl.at(27); d.click(56, 40)          # File > Add images…
    tl.at(29); d.shot("/tmp/gui/c01_s3.png")
    tl.at(31); pick(d, S / "alba.png")
    tl.at(41); d.click(20, 12); tl.at(43); d.click(56, 62)          # File > Add folder…
    tl.at(47); pick_folder(d)
    tl.at(56); d.shot("/tmp/gui/c01_s4.png")
    tl.at(60); d.click(135, 330); tl.at(63); d.click(72, 84); tl.at(66); d.click(24, 84)   # select, Down, Up
    tl.at(69); d.click(88, 61)                                      # Remove
    tl.at(73); d.click(150, 61)                                     # Clear
