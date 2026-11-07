# LegatorOCR command-line image (server / batch use). NOT BUILT AS AN IMAGE: on 2026-11-07 a Docker daemon
# started in the work environment but the registry (registry-1.docker.io) was refused by its network policy,
# so the base images could not be pulled. Checked instead: the files this Dockerfile copies (Cargo.toml,
# Cargo.lock, crates/, tools/) are enough for `cargo build --release --locked -p legatorocr-cli`. Not checked:
# the Rust version of the rust:1 image against rust-version in Cargo.toml, the apt package names, the
# runtime of the image. Build it once and run `docker run --rm legatorocr --doctor` before relying on it.
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
