# Romanian CTC heads

`crnn_mobilenet_v3_small.npz` and `crnn_vgg16_bn.npz` hold only the new final linear layer (vocabulary in `vocab.txt`, adds ă ș ț Ă Ș Ț)
trained by `tools/ocr-train/romanian_head.py` on synthetic words. The backbone is **not** in this repository: it is taken at run time
from the docTR / OnnxTR recognisers (Apache-2.0, Mindee; <https://github.com/mindee/doctr>, <https://github.com/felixdittrich92/OnnxTR>)
downloaded by `scripts/get-romanian-doctr-models.sh`. The heads derive from those weights; the original training data is not
documented by the maintainers (see `docs/audit/assets-licences.md`, risk R5). Measured results: ROADMAP 4.32 and 4.33.
