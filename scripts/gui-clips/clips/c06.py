DURATION = 89.5
FILES = ["negativ.png", "perforat.png", "foto-telefon.jpg"]
WARMUP = 14
def act(tl, d, sc):
    d.click(1039, 107); d.click(1053, 130); d.click(1038, 174)        # fold Input, Geometry, Output
    tl.at(1.5); d.click(1013, 199)                                    # unfold Clean-up
    tl.at(7); d.click(1031, 222); tl.at(13); d.click(1031, 222)        # invert negative scans
    tl.at(15); d.click(1031, 245); tl.at(21); d.click(1031, 245)       # auto contrast
    tl.at(23); d.click(1031, 268); tl.at(28); d.click(1031, 268)       # show-through
    tl.at(29); d.click(1031, 291); tl.at(33); d.click(1031, 291)       # descreen
    tl.at(35); d.click(135, 430); tl.at(37); d.click(1031, 370)        # page 2, punch holes
    tl.at(44); d.click(1031, 370)
    tl.at(46); d.click(135, 650); tl.at(48); d.click(1031, 314)        # page 3, fix perspective
    tl.at(54); d.click(1031, 337)                                      # wipe the background
    tl.at(62); d.click(1031, 393)                                      # remove fingers
    tl.at(69); d.click(1150, 416); tl.at(72); d.click(1130, 462)       # drop colour list
    tl.at(74); d.drag(1086, 462, 1135, 462, 10)                        # sharpen
    tl.at(78); d.click(1180, 439); d.type_text("10,10,30,30", 0.1)     # erase zones
