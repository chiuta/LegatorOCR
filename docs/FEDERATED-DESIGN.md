# Federated model improvement: design note (nothing is built)

Status: **idea only**, written 2026-11-05. No code, no server, no protocol exists. This note says what
would have to be true before building it, so that the idea is not mistaken for a feature.

## The idea
Each installation fine-tunes the OCR model locally on the lines its user verified (`--verify-pack`,
`tools/ocr-train/adapt_nightly.sh`) and shares only a **weight difference** (the new weights minus the
old ones), never pages, text or images. A coordinator averages the differences of many users into
a new community model.

## Why it is not trivial
- **Nothing leaves the machine today** (`--offline` refuses the two features that use the network). A
  sharing step is a new network feature and must be opt-in, off by default, and visible.
- **Weight differences can leak.** A difference computed from a few hundred lines of one document can
  reveal parts of that document (gradient inversion is a known attack on federated learning). Safe
  sharing would need clipping and noise (differential privacy) or secure aggregation, and both cost
  accuracy. Not evaluated here.
- **Poisoning.** One bad or hostile participant can push the average. Needs per-update checks (the gate of
  `adapt_nightly.sh` on a shared held-out set) and a way to drop a participant.
- **Evidence that it helps is missing.** In this repository fine-tuning on one book's pages gave
  no gain on that book (ROADMAP 4.50) and a small loss elsewhere; human-verified lines have not yet
  been collected and measured (4.52). Averaging updates of models that each drift towards their own
  book may lower accuracy for everybody.
- **Infrastructure and trust.** A server, a signing key for models, versioning, a way to roll back, and
  someone who runs it. The project is clean-room and trade-free: the coordinator should be
  self-hostable.

## What would be a first, honest step
1. Collect real verified lines from several books with `--verify-pack`; measure, on held-out pages, whether
   fine-tuning on **other** people's books helps a new book (the question federation depends on).
2. Only if yes: a manual version first (users send a weight-difference file to the maintainer by e-mail,
   the maintainer averages and gates it), before any protocol.
3. Only then: automation, with the privacy and poisoning measures above.
