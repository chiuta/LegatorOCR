# LegatorOCR command-line image (server / batch use). UNTESTED: written on 2026-11-06 where no Docker
# daemon was available; build it once and run `docker run --rm legatorocr --doctor` before relying on it.
#
#   docker build -t legatorocr .
#   docker run --rm -v "$PWD":/work legatorocr scan-*.png --pdf /work/book.pdf --ocr --lang ron
#
# Only the command-line program is built (no window). Network use is off in the container only
# if you add --network none; the program itself needs the network for --timestamp and --scan-escl.

FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY tools ./tools
RUN cargo build --release --locked -p legatorocr-cli

FROM debian:bookworm-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      tesseract-ocr tesseract-ocr-ron tesseract-ocr-eng tesseract-ocr-osd \
      tesseract-ocr-fra tesseract-ocr-ita tesseract-ocr-spa tesseract-ocr-por \
      tesseract-ocr-deu tesseract-ocr-hun \
      poppler-utils qpdf ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/legatorocr /usr/local/bin/legatorocr
RUN useradd --create-home --uid 10001 legator
USER legator
WORKDIR /work
ENTRYPOINT ["legatorocr"]
CMD ["--help"]
