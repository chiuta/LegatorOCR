# Notice on the removed `mobila1960_*_hand_reference.txt` files

Until 0.2.0-pre.66 this folder held five hand transcriptions of pages of a 1960 Romanian textbook ("Tehnologia producerii mobilei"), made to measure OCR recall. They reproduced about 900 words of that book and its rights holder is unknown, so they were removed from the tree in 0.2.0-pre.67. They remain in the git history of earlier commits.

The scripts `mobila1960_score.py`, `mobila1960_word_recall.py` and `book_vote.py` are kept as method only: they expect reference files that you supply (a hand transcription of part of a page, one `.txt` per page). The results they produced are recorded in `ROADMAP.md` (4.15, 4.16); those numbers can no longer be reproduced from this repository.
