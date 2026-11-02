B=/home/claude/legatorocr/target/release/legatorocr
for t in sauvola wolf niblack nick bradley otsu auto; do
  mkdir -p thr_$t
  for p in 4 7 8 13 19 21 27 31 38 45 50 52; do
    $B img/p$p.png --mode bw --threshold $t --pdf /tmp/pf/e.pdf --ocr --lang ron --format none --txt thr_$t/p$p.txt >/dev/null 2>&1
  done
done
