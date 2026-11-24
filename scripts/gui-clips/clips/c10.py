DURATION = 44.8
FILES = ["alba.png", "curbat.png", "negativ.png", "perforat.png", "lateral.png", "foto-telefon.jpg"]
WARMUP = 14
def act(tl, d, sc):
    tl.at(0.4); d.click(147, 12)                                  # All options…
    tl.at(4); d.click(70, 132)                                    # unfold "Pages and order"
    tl.at(12); d.click(170, 100); d.type_text("burst", 0.12)       # the search box
    tl.at(16); d.key("a", ["Control_L"]); d.key("BackSpace")
    tl.at(17.5); d.click(48, 269)                                 # --dry-run
    tl.at(19); d.click(48, 193)                                   # --duplex-adf
    tl.at(21); d.click(338, 100)                                  # Only chosen
    tl.at(25); d.click(76, 612)                                   # Copy command
    tl.at(28.5); d.click(156, 612)                                # Run
    tl.at(36); d.click(533, 100)                                  # Clear choices
    tl.at(40); d.click(858, 33)                                   # close
