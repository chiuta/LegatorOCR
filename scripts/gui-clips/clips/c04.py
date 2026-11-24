DURATION = 79.0
FILES = ["curbat.png", "dublu-strâmb.png"]
WARMUP = 14
def act(tl, d, sc):
    tl.at(0.5); d.click(657, 43)                                        # Draw zones on
    tl.at(6); d.click(766, 43); tl.at(8); d.drag(330, 320, 470, 380, 14)   # Erase
    tl.at(13); d.click(827, 43); tl.at(15); d.drag(340, 450, 540, 570, 14) # Picture
    tl.at(21); d.click(902, 43); tl.at(23); d.drag(470, 250, 600, 330, 14) # No picture
    tl.at(28); d.click(400, 345, 3); tl.at(31); d.click(393, 135)       # right-click removes one; Undo
    tl.at(33); d.click(428, 135); tl.at(35); d.click(322, 135)          # Redo; Clear zones
    tl.at(39); d.click(315, 65); tl.at(40.5); d.drag(300, 330, 610, 340, 16)  # Guide line
    tl.at(43); d.click(1031, 452)                                       # Straighten curved lines
    tl.at(46); d.click(398, 65); tl.at(48); d.drag(300, 230, 610, 670, 16)    # Work area
    tl.at(52); d.click(135, 330)                                        # second page
    tl.at(53); d.click(475, 65); tl.at(55); d.click(450, 400)            # Cut line
    tl.at(62); d.click(657, 43); d.click(287, 823)                       # Content box
    tl.at(66); d.glide(520, 300, 20); tl.at(70); d.glide(560, 600, 20)
