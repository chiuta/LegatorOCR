# Model files: provenance

| File | Built in? | Trained on | Rights status |
|---|---|---|---|
| `multi.lnm` | yes (compiled in) | synthetic lines rendered from third-party fonts (not embedded; see `THIRD_PARTY_LICENSES.md`), then fine-tuned on real scanned lines from a magazine and a 1960 textbook; labels from an ABBYY text layer and from agreement between Tesseract and the model | scans' rights **not documented**; see `docs/clean-room/PROVENANCE.md` |
| `candidate-books.lnm` | no (tested, not adopted: ROADMAP 4.65/4.70) | further fine-tuning on book pages | rights of the source PDFs **not documented** |
| `candidate-digital.lnm` | no (tested, not adopted: ROADMAP 4.49/4.63) | the two digital PDFs a user supplied (`Bro_ura_Jocuri_2017.pdf`, a second PDF) | rights of the source PDFs **not documented** |

Model weights are numeric parameters, not copies of the scans, but whether training on a work without a licence is permitted depends on the jurisdiction (in the EU, text-and-data-mining exceptions under Directive 2019/790 art. 3-4, with opt-out for commercial use). Confirm before redistributing the models commercially. The candidate files are not used by any build; remove them if you cannot document their sources.
