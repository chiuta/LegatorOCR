# Lot mare pe mai multe calculatoare

Pe fiecare calculator, aceeași listă de fișiere și altă felie:

```
legatorocr scans/*.png --shard 1/3 --profile book-bw --pdf part1.pdf -o pages1
legatorocr scans/*.png --shard 2/3 --profile book-bw --pdf part2.pdf -o pages2
legatorocr scans/*.png --shard 3/3 --profile book-bw --pdf part3.pdf -o pages3
legatorocr --merge-pdf part1.pdf part2.pdf part3.pdf --pdf carte.pdf
```

Ca să ții memoria mică pe un singur calculator: `--max-memory 500`. Pentru fișiere necunoscute sau ostile adaugă `--isolate-decoders`.
