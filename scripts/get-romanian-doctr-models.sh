#!/usr/bin/env bash
# ROADMAP 4.33: fetch the original docTR/OnnxTR ONNX models from the OnnxTR GitHub release assets and build the
# Romanian-head recogniser (tools/ocr-train/romanian_head.py splice). Prints the command line to use.
#
#   scripts/get-romanian-doctr-models.sh [mobilenet|resnet] [DIR]
#
#   mobilenet (default): db_mobilenet_v3_large + crnn_mobilenet_v3_small   (8 + 16 MB, about 2.5 s per page on 2 cores)
#   resnet             : db_resnet50 + crnn_vgg16_bn                        (about 100 + 63 MB, much slower, a little more accurate)
#   DIR                : where the models go (default ./romanian-doctr-models)
#
# Needs: curl, python3 with onnx and numpy (for the splice), and at run time onnxruntime, numpy, Pillow.
# Only the original models are downloaded (GitHub release assets, Apache-2.0 weights of docTR/OnnxTR); the new
# recogniser head (139 KB) comes from this repository. Nothing is uploaded.
set -euo pipefail
FAMILY="${1:-mobilenet}"
DIR="${2:-./romanian-doctr-models}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HEADS="$HERE/../tools/ocr-train/romanian_head"
BASE="https://github.com/felixdittrich92/OnnxTR/releases/download"
case "$FAMILY" in
  mobilenet)
    DET="$BASE/v0.2.0/db_mobilenet_v3_large-4987e7bd.onnx"
    REC="$BASE/v0.0.1/crnn_mobilenet_v3_small-bded4d49.onnx"
    ARCH=crnn_mobilenet_v3_small ;;
  resnet)
    DET="$BASE/v0.0.1/db_resnet50-69ba0015.onnx"
    REC="$BASE/v0.7.1/crnn_vgg16_bn-743599aa.onnx"
    ARCH=crnn_vgg16_bn ;;
  *) echo "unknown family '$FAMILY' (mobilenet or resnet)" >&2; exit 2 ;;
esac
mkdir -p "$DIR"
DIR="$(cd "$DIR" && pwd)"
for u in "$DET" "$REC"; do
  f="$DIR/$(basename "$u")"
  if [ ! -s "$f" ]; then
    echo "downloading $(basename "$u")" >&2
    curl -fL --retry 3 -o "$f.part" "$u"
    mv "$f.part" "$f"
  fi
done
[ -f "$HEADS/$ARCH.npz" ] || { echo "missing $HEADS/$ARCH.npz (run from a full checkout)" >&2; exit 1; }
ROHEAD_WORK="$DIR" python3 "$HERE/../tools/ocr-train/romanian_head.py" splice "$ARCH" "$HEADS/$ARCH.npz" "$DIR/${ARCH}_ro.onnx" "$DIR/vocab_ro.txt" >&2
cat <<EOF

Done. Use it with:

  export LEGATOROCR_ONNX_FAMILY=doctr
  export LEGATOROCR_ONNX_DET="$DIR/$(basename "$DET")"
  export LEGATOROCR_ONNX_REC="$DIR/${ARCH}_ro.onnx"
  export LEGATOROCR_ONNX_DICT="$DIR/vocab_ro.txt"
  legatorocr scan.png --pdf out.pdf --ocr --engine tesseract --tesseract $HERE/engine-onnx.py
EOF
