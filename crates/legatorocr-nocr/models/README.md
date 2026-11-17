# Model files: provenance

| File | Built in? | Trained on | Rights status |
|---|---|---|---|
| `multi.lnm` | yes (compiled in) | synthetic lines rendered from third-party fonts (not embedded; see `THIRD_PARTY_LICENSES.md`), then fine-tuned on real scanned lines from a magazine and a 1960 textbook; labels from an ABBYY text layer and from agreement between Tesseract and the model | scans' rights **not documented**; see `docs/clean-room/PROVENANCE.md` |

Model weights are numeric parameters, not copies of the scans, but whether training on a work without a licence is permitted depends on the jurisdiction (in the EU, text-and-data-mining exceptions under Directive 2019/790 art. 3-4, with opt-out for commercial use). Confirm before redistributing the models commercially. The two candidate models (`candidate-books.lnm`, `candidate-digital.lnm`), which were trained on PDFs of third parties and were never part of a build, were removed from the tree in 0.2.0-pre.67 because their sources could not be documented. They remain in the git history; ROADMAP 4.49, 4.63, 4.65 and 4.70 describe the experiments.
