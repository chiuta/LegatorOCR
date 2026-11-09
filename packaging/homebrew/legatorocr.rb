# Homebrew formula (source build from the tagged release). Not submitted to a tap yet;
# replace the sha256 when the tag is published. `:recommended` is allowed in a tap but not in
# Homebrew/homebrew-core; core also calls a bare `--version` test weak (Formula Cookbook, read 2026-11-04).
class Legatorocr < Formula
  desc "Scan post-processing, PDF export and OCR"
  homepage "https://github.com/chiuta/LegatorOCR"
  url "https://github.com/chiuta/LegatorOCR/archive/refs/tags/v0.2.0.tar.gz"
  sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  license "MIT"

  depends_on "rust" => :build
  depends_on "tesseract" => :recommended

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/legatorocr-cli")
  end

  test do
    assert_match "legatorocr", shell_output("#{bin}/legatorocr --version")
  end
end
