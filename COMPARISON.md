# LegatorOCR compared with ScanTailor and its forks

Scope: feature lists and release notes only (no source code read). Status of LegatorOCR is as of v0.2 development;
"✔" = implemented and tested here, "◐" = partly, "✘" = not implemented.

Sources read for this comparison:

- ScanTailor *Experimental* release notes (ImageProcessing-ElectronicPublications):
  <https://github.com/ImageProcessing-ElectronicPublications/scantailor-experimental/releases> — newest "nail"
  1.2026.10.10 (21 Oct) back to "sky" 1.2025.05.05.
- Classic ScanTailor workflow stages (fix orientation → split pages → deskew → select content → margins/page layout →
  output), which every fork shares.

| Capability | Classic ScanTailor workflow | Experimental fork (per its notes) | LegatorOCR |
|---|---|---|---|
| Orientation fix | manual, per page | – | ✔ auto (profile energy) + Tesseract OSD + manual |
| Page split | ✔ | – | ✔ auto / middle / off, per page override |
| Deskew | ✔ auto + manual | – | ✔ auto + manual |
| Content box | ✔ auto + hand-edit | thresholds tuned ("Select content") | ✔ auto, shadow/edge rejection; per-page numeric nudge; ✘ drag-editing |
| Margins / page size / alignment | ✔ | manual scaling, "Sub" layout mode | ✔ margins, fixed size, alignment, **book-wide uniform size** |
| Binarisation methods | Otsu / Sauvola / Wolf | many: Niblack, Sauvola, Wolf, N.I.C.K., Gatos, Singh, Fox, Dynamic Window, OBBPM, LAAB, LCaM … | ◐ six + **auto per page** |
| Despeckle | ✔ | synced with original | ✔ |
| Picture zones (mixed output) | ✔ hand-drawn zones | – | ◐ **automatic** detection (mid-tone share) + typed zones (`--picture-zone`) + **mouse-drawn zones** in the GUI (followed through rotate / split / deskew) |
| Fill / erase zones | ✔ | – | ✔ typed rectangles (mm) or mouse-drawn per sheet |
| Dewarping | ✔ semi-automatic, control points | "dewarp correction parameters" | ◐ automatic from text lines; vertical bend only; no control points |
| Lighting normalisation | colour/illumination options | colour corrections, curves, "RIS undefect", EM denoiser, deBlur5 | ◐ background normalisation; ✘ denoise/deblur filters |
| Output resolution | ✔ | – | ✔ down-sampling to `--output-dpi` |
| Project files | `.ScanTailor` (XML) | relative paths in project files | ✔ plain-text `.lgr`, relative paths |
| PDF with text layer | ✘ (images only) | ✘ | ✔ own writer + Tesseract OCR layer |
| Compact PDF | – | – | ✔ CCITT G4, **hybrid** layered pages (mask + low-res background + patches) |
| Multi-page TIFF input | ✔ | – | ✔ |
| PDF input | – | – | ✔ scanned pages incl. layered/MRC, CCITT G3/G4, JBIG2 generic/symbol/text/refinement/halftone, native JPEG 2000, native decryption (RC4/AES), external-renderer fallback for the rest |
| Punch holes, fingers, ruled lines, colour drop-out, negatives, levels, sharpen | partial (fill zones) | partial | ✔ automatic, off by default |
| Profiles | – | – | ✔ built-in + user, CLI and GUI |
| Text / hOCR sidecar, PDF metadata | ✘ | ✘ | ✔ |
| Show-through, halftone smoothing | ✘ | partial (filters) | ✔ |
| Duplicate pages, separator-sheet splitting, target PDF size | ✘ | ✘ | ✔ (blank pages **and bar-code sheets** (Code 128, EAN-13, ITF, Code 39) as separators, with `--make-separator` to print them) |
| OCR on enlarged low-dpi pages | – | – | ✔ |
| Perspective correction of phone photos | – | – | ◐ automatic quad detection, conservative |
| HTML contact-sheet report, multi-page G4 TIFF | ✘ | ✘ | ✔ |
| Outlier page report | – | – | ✔ `--report-outliers` |
| GUI languages | many | many | 8 (ro en fr it es pt de hu) |
| Platforms | Win / macOS / Linux | same | same (Windows/macOS from CI, unverified here) |

## Where LegatorOCR deliberately differs

- **Analysis at ≈150 dpi.** A 600 dpi scan is orientation-, skew-, content- and picture-analysed on a 1/16-pixel copy; only
  the final cut/threshold runs at full resolution.
- **Binarise first, then cut.** Thresholding the whole deskewed scan before cropping avoids the black frame an adaptive
  threshold paints around a window on uneven paper.
- **Hybrid output.** Text as a CCITT G4 stencil, pictures as full-resolution patches, paper as a ≈100 dpi JPEG: colour book
  pages around 60–100 KB.
- **Statistics from sliding column sums**, so every local threshold needs O(width) memory instead of an integral image.

## Not done (honest list)

A draggable content box (numeric nudge only), dewarp control points, a native (non-Tesseract) OCR engine, table / column structure in the Word export, JBIG2 Huffman coding, certified PDF/A (veraPDF validates our generated sample files against PDF/A-2b in CI; that covers the samples, not every input), the extra binarisers of the Experimental
fork, EM denoise / deblur filters, and any run of the GUI or the Windows / macOS builds by the author (CI now smoke-tests the packaged command-line binary on every release target).

Measured, not claimed: `docs/OCR-BENCH.md` (synthetic OCR accuracy, 8 languages × 3 conditions).
