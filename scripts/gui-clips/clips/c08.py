DURATION = 43.5
FILES = ["dublu-strâmb.png"]
def act(tl, d, sc):
    tl.at(0.5); d.click(1127, 61)                 # open the Profile list
    tl.at(3); d.glide(1127, 180, 25); tl.at(7); d.glide(1127, 250, 20)
    tl.at(11); d.click(1100, 111)                 # choose book-small-color
    tl.at(14); d.shot("/tmp/gui/c08_choose.png")
    tl.at(19); d.click(1103, 37); tl.at(22); d.click(1158, 37); tl.at(25); d.click(1103, 37)
    tl.at(28); d.click(1060, 84); d.type_text("carte-mea", 0.15)
    tl.at(33); d.click(1167, 84)
    tl.at(36); d.click(1127, 61); tl.at(38); d.glide(1127, 290, 20); d.shot("/tmp/gui/c08_saved.png")
    tl.at(41); d.key("Escape")
