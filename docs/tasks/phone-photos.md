# Phone photos → clean pages

```
legatorocr photos/*.jpg --profile phone-photos --pdf pagini.pdf -o pages
```

- `--profile phone-photos` levels the lighting, widens the skew range and removes fingers.
- Several shots of the same page can be merged first: `legatorocr --fuse-frames page.png shot1.jpg shot2.jpg shot3.jpg`
  (helps with strong sensor noise; on moderate noise it does not).
