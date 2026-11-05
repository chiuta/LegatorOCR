#!/bin/bash
# Overnight adaptation with a gate (idea 4 of the 2026-11-05 list).
# Usage: adapt_nightly.sh VERIFIED.json [MORE.json ...]
#   env: BASE_REAL  tsv of lines kept in the mix (default /tmp/real2.tsv)   STEPS (default 4000)
#        WORK       work folder (default /tmp/nightly)   MODEL  current model (default the compiled-in one)
# Steps: import the answers of --verify-pack pages -> fine-tune from MODEL (BN frozen) -> gate:
#   (1) line CER on held-out lines must not get worse (held_cer.py, HELD_PAGES / REAL_HELD),
#   (2) the 12 magazine pages (eval_real.py, ALL12) must not get worse by more than 0.15 points.
# Accepted: the model is copied to crates/legatorocr-nocr/models/candidate-<date>.lnm (never over multi.lnm).
# Rejected: nothing is copied, exit 1. Needs the same files as train.py (/tmp/ocr_eval/img etc.).
set -e
HERE=$(cd "$(dirname "$0")" && pwd); ROOT=$HERE/../..
WORK=${WORK:-/tmp/nightly}; STEPS=${STEPS:-4000}; MODEL=${MODEL:-$ROOT/crates/legatorocr-nocr/models/multi.lnm}
BASE_REAL=${BASE_REAL:-/tmp/real2.tsv}
mkdir -p "$WORK"
python3 "$HERE/import_verify.py" "$@" "$WORK/lines" "$WORK/verified.tsv"
cat "$BASE_REAL" "$WORK/verified.tsv" > "$WORK/mix.tsv"
cd "$HERE"
REAL=$WORK/mix.tsv REALP=0.6 FREEZE_BN=1 LR=8e-4 WARM=0.1 TT=2 NW=1 INIT_LNM=$MODEL \
  python3 train.py "$WORK/run" "$STEPS" > "$WORK/train.log" 2>&1
NEW=$WORK/run/model.lnm
python3 gate.py "$MODEL" "$NEW" "$WORK/mix.tsv" && {
  OUT=$ROOT/crates/legatorocr-nocr/models/candidate-$(date +%F).lnm
  cp "$NEW" "$OUT"; echo "accepted: $OUT"; } || { echo "rejected by the gate (see above); nothing copied"; exit 1; }
