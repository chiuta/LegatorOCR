DURATION = 101.1
FILES = ["perforat.png"]
WARMUP = 12
def act(tl, d, sc):
    tl.at(0.5); d.glide(1100, 107, 15)
    tl.at(3); d.click(1143, 130); d.key("a", ["Control_L"]); d.type_text("300", 0.1); d.key("Tab")
    tl.at(16); d.click(1031, 153); tl.at(19); d.click(1031, 153); tl.at(21); d.click(1031, 176); tl.at(24); d.click(1031, 176)
    tl.at(26); d.click(1137, 199); tl.at(28); d.glide(1137, 250, 12); tl.at(30); d.key("Escape")
    tl.at(32); d.click(1031, 245); tl.at(35); d.click(1031, 245); tl.at(37); d.drag(1132, 268, 1160, 268, 10)
    tl.at(42); d.click(1218, 291); d.key("a", ["Control_L"]); d.type_text("8", 0.1); d.key("Tab")
    tl.at(47); d.click(1134, 314); tl.at(50); d.key("Escape")
    tl.at(52); d.click(1108, 383); tl.at(57); d.click(1184, 383); tl.at(63); d.click(1263, 383); tl.at(68); d.click(1325, 383)
    tl.at(74); d.click(1050, 383)
    tl.at(76); d.click(1031, 406); tl.at(83); d.click(1031, 452); tl.at(88); d.click(1031, 406)
    tl.at(91); d.click(1103, 37); tl.at(94); d.click(1158, 37); tl.at(97); d.click(1103, 37)
