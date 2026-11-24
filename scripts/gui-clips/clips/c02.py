DURATION = 98.5
FILES = ["dublu-strâmb.png", "perforat.png", "alba.png", "negativ.png", "lateral.png", "foto-telefon.jpg", "curbat.png"]
WARMUP = 25
ITEM = [205, 330, 455, 580, 705, 830]
def act(tl, d, sc):
    d.click(112, 139)                       # small thumbnails: all seven fit
    tl.at(1); d.click(135, ITEM[0])
    tl.at(6); d.click(321, 43)              # Side by side
    tl.at(9); d.click(403, 43); tl.at(13); d.click(478, 43)   # Original, Processed
    tl.at(17); d.click(548, 43); tl.at(19); d.drag(600, 420, 760, 420, 25); tl.at(23); d.drag(760, 420, 520, 420, 25)
    tl.at(26); d.click(602, 43); tl.at(31); d.click(602, 43)  # Fit off / on
    tl.at(34); d.click(321, 43)
    tl.at(36); d.click(135, ITEM[1]); tl.at(41); d.click(135, ITEM[2]); tl.at(46); d.click(135, ITEM[3])
    tl.at(52); d.click(135, ITEM[4]); tl.at(55); d.click(135, ITEM[5])
    tl.at(57)
    for k in ["Up", "Up", "Down", "Next", "Home", "End", "Home"]:
        d.key(k); tl.at(tl.now() + 1.3)
    tl.at(68); d.key("k", ["Control_L"]); tl.at(70); d.type_text("rot", 0.2); tl.at(75); d.key("Escape")
    tl.at(78); d.click(135, ITEM[0])
    tl.at(79); d.key("n"); tl.at(82); d.key("n")
    tl.at(85); d.key("k", ["Control_L"]); tl.at(87); d.type_text("flag", 0.2); tl.at(91); d.key("Return")
