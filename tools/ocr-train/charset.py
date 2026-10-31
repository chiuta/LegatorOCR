# Character set of the native recogniser: index 0 is the CTC blank, 1 is the word space.
# Must stay identical to CHARSET in crates/legatorocr-nocr/src/model.rs (checked by a test).
LETTERS = (
    "abcdefghijklmnopqrstuvwxyz"
    "ABCDEFGHIJKLMNOPQRSTUVWXYZ"
    "ăâîșțĂÂÎȘȚ"          # Romanian (comma-below)
    "àâçéèêëîïôùûüÿœæÀÂÇÉÈÊËÎÏÔÙÛÜŸŒÆ"  # French
    "áíóúñÁÍÓÚÑ¿¡"         # Spanish / Portuguese
    "ãõÃÕ"                 # Portuguese
    "äöüßÄÖÜẞ"             # German
    "őűŐŰ"                 # Hungarian
    "ìòÌÒ"                 # Italian
)
DIGITS = "0123456789"
PUNCT = ".,;:!?-–—'’\"“”„«»()[]/%&@#+=*€$§°…_"
CHARS = []
for c in " " + LETTERS + DIGITS + PUNCT:
    if c not in CHARS:
        CHARS.append(c)
CHARSET = "".join(CHARS)  # position i here is class i+1 (class 0 = blank)
