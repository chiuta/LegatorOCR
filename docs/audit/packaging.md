# Packaging audit (read 2026-11-04)

Sources: GitHub REST API `repos/chiuta/LegatorOCR/releases`, release downloads via curl, learn.microsoft.com (winget manifest), endoflife.date (freedesktop-sdk), docs.flathub.org, Homebrew Formula Cookbook (mirror), Arch wiki Rust package guidelines (2026-04 mirror).

## Releases (reality)
- 15 releases, all pre-releases (`v0.2.0-pre.1` .. `v0.2.0-pre.15`). **No `v0.2.0` tag/release exists** (`archive/refs/tags/v0.2.0.tar.gz` gives 404). Workspace version is `0.2.0-pre.15`.
- Each release: 4 archives (aarch64/x86_64 apple-darwin `.zip`, aarch64/x86_64 linux-gnu `.tar.gz`, x86_64 windows-msvc `.zip`), a `.sha256` per archive, `SHA256SUMS.txt`, SBOM `.cdx.json`. No `.deb`, no portable-folder zip is attached.
- pre.15 Windows zip: 15,816,834 bytes, SHA-256 `e812a912...7c89`, computed by me after download = matches the API digest and `SHA256SUMS.txt`. The zip holds ONE top-level folder `LegatorOCR-<ver>-x86_64-pc-windows-msvc/` with `legatorocr.exe`, `legatorocr-gui.exe`, LICENSE, README, THIRD_PARTY_LICENSES, LEXICONS-NOTICE.
- Source tarball of a tag unpacks to `LegatorOCR-<ver>/` (checked on pre.15), so the PKGBUILD `cd "LegatorOCR-$pkgver"` is right. pre.15 tarball SHA-256 `8ace4c2f...9142` (GitHub regenerates tarballs; do not rely on it).

## Per file
| File | Repo said | Reality / rule now | Action |
|---|---|---|---|
| winget | `ManifestVersion 1.6.0`; `RelativeFilePath: legatorocr.exe` | Current schema 1.12.0 (Microsoft docs). The zip has a top-level folder, so the exe path was wrong | set 1.12.0, path with folder prefix, `$schema` comment; hash stays 000... (no v0.2.0 asset) |
| winget | singleton | allowed for one installer + one locale; winget-pkgs tooling usually emits 3 files | noted in comment |
| homebrew | `cargo install --locked --root prefix --path ...` | Cookbook lists `std_cargo_args` (= --locked --root --path) | switched to `std_cargo_args(path: ...)`; `:recommended` only allowed in taps, not core; `--version` test called weak in core |
| aur | `cargo build --release --locked`, `cargo test --release` | Arch Rust guidelines: `prepare()` with `cargo fetch --locked`, `build --frozen --release`, `check()` without `--release`, set `RUSTUP_TOOLCHAIN`, `CARGO_TARGET_DIR` | rewritten; `sha256sums=SKIP` stays until tag exists |
| flatpak | runtime 24.08 | 24.08 EOL 2026-09-08; current 26.08 (25.08 also maintained). Flathub refuses EOL runtimes and "console software"; metainfo is mandatory; no network during build | runtime 26.08, header comment: CLI-only skeleton would be refused, GUI flatpak needed |
| debian control/rules | `rustc (>= 1.95)` | matches `rust-version = "1.95"` | none |
| build-deb.sh | - | `sh -n` and `shellcheck` clean; not attached to releases | none |
| licence fields | MIT everywhere | Cargo.toml and LICENSE are MIT | none (not changed) |
| README install | Releases page, archive names, tesseract commands | archive types match the table; `UB-Mannheim.TesseractOCR` seen only in third-party winget indexes; brew `tesseract` and `tesseract-lang` formulae exist in homebrew-core (HTTP 200) | none |

## Syntax checks run
`ruby -c` (OK), `bash -n` PKGBUILD (OK), `sh -n` + shellcheck build-deb.sh (clean), PyYAML parse winget + flatpak (OK).

## Could not verify
- Any real run of winget, brew, flatpak-builder, makepkg/namcap, lintian (not installed/possible here).
- winget schema field-level validation against the JSON schema (host not reachable); winget-pkgs submission rules (singleton acceptance).
- That `org.freedesktop.Sdk.Extension.rust-stable` is published for 26.08 and ships rustc >= 1.95.
- Flathub verification of app-id `tf.trom.LegatorOCR` (needs control of trom.tf).
- Homebrew rules were read from a mirror of the Cookbook, not docs.brew.sh (blocked); Arch rules from a 2026-04 wiki mirror.
- Hashes for the final tag: impossible before `v0.2.0` is published.
