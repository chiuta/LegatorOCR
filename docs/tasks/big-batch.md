# Big batch on several machines

On each machine, the same list of files and a different slice:

```
legatorocr scans/*.png --shard 1/3 --profile book-bw --pdf part1.pdf -o pages1
legatorocr scans/*.png --shard 2/3 --profile book-bw --pdf part2.pdf -o pages2
legatorocr scans/*.png --shard 3/3 --profile book-bw --pdf part3.pdf -o pages3
legatorocr --merge-pdf part1.pdf part2.pdf part3.pdf --pdf carte.pdf
```

To keep memory low on one machine: `--max-memory 500`. For hostile or unknown files add `--isolate-decoders`.
