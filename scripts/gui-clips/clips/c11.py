DURATION = 55.9
FILES = ["dublu-strâmb.png", "alba.png"]
WARMUP = 14
def act(tl, d, sc):
    tl.at(0.5); d.click(70, 12)                                   # Language menu
    tl.at(4); d.glide(70, 120, 12)
    tl.at(7); d.click(70, 82)                                     # Français
    tl.at(12); d.click(80, 12); tl.at(14); d.click(70, 60)        # back to English
    tl.at(17); d.click(211, 12); tl.at(20); d.click(222, 121)     # Help ▸ About
    tl.at(25); d.click(334, 40)
    tl.at(28); d.click(20, 12); tl.at(30); d.click(75, 250)       # File ▸ Optional modules…
    tl.at(36); d.click(546, 42)
    tl.at(38); d.click(1031, 245); tl.at(41); d.key("z", ["Control_L"]); tl.at(45); d.key("y", ["Control_L"])
    tl.at(50); d.glide(700, 400, 20)
