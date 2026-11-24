DURATION = 82.0
FILES = ["perforat.png", "alba.png"]
WARMUP = 14
def act(tl, d, sc):
    d.click(1039, 107); d.click(1053, 130); d.click(1038, 174)   # fold the first three and the clean-up
    tl.at(2); d.shot("/tmp/gui/c07_a.png")
    tl.at(5); d.drag(1180, 245, 1120, 245, 12); tl.at(9); d.click(1115, 268); d.key("a", ["Control_L"]); d.type_text("150", 0.1); d.key("Tab")
    tl.at(14); d.click(1031, 304); tl.at(18); d.click(1031, 327)
    tl.at(21); d.click(1031, 350)
    tl.at(26); d.click(1031, 373); tl.at(29); d.click(1031, 396)
    tl.at(33); d.click(1031, 419); tl.at(36); d.click(1031, 442)
    tl.at(40); d.click(1270, 465); d.key("a", ["Control_L"]); d.type_text("3", 0.1); d.key("Tab")
    tl.at(45); d.click(1140, 488); tl.at(47); d.key("Escape")
    tl.at(50); d.click(1190, 511); d.type_text("Alexio", 0.1); tl.at(52); d.click(1250, 534)
    tl.at(55); d.click(1031, 603); tl.at(57); d.click(1050, 670)
    tl.at(60); d.scroll(1180, 600, 8); tl.at(62); d.shot("/tmp/gui/c07_b.png")
    tl.at(70); d.scroll(1180, 600, 8); tl.at(72); d.shot("/tmp/gui/c07_c.png")
