DURATION = 75.3
FILES = ["lateral.png", "dublu-strâmb.png"]
WARMUP = 14
def act(tl, d, sc):
    tl.at(0.3); d.click(135, 205)                       # sideways scan first
    tl.at(5); d.click(396, 66)                          # Rotate left 90
    tl.at(11); d.click(509, 66)                         # Rotate right 90
    tl.at(16); d.click(135, 330)                        # the crooked double sheet
    tl.at(22); d.click(641, 66); tl.at(27); d.click(688, 66); tl.at(30); d.click(688, 66)
    tl.at(34); d.click(890, 112); tl.at(36); d.click(890, 112)   # Manual skew
    tl.at(38); d.click(555, 112)                        # Split this sheet
    tl.at(41); d.glide(555, 190, 15)
    tl.at(42); d.key("Escape")
    tl.at(45); d.click(750, 112); tl.at(48); d.key("Escape")      # Output of this page
    tl.at(49); d.click(468, 158); d.key("ctrl+a" if False else "a", ["Control_L"]); d.type_text("2", 0.1); d.key("Tab")
    tl.at(55); d.click(550, 158); d.key("a", ["Control_L"]); d.type_text("3", 0.1); d.key("Tab")
    tl.at(63); d.click(393, 135); tl.at(66); d.click(428, 135); tl.at(69); d.click(504, 135)
