# Packaging files (roadmap 7.2)

Source-build recipes for the usual package channels. **None has been built by its package manager
or submitted**; each needs the tag to be published first (checksums are placeholders) and
a human to open the submission under their own account.

| File | Channel | Checked how |
|---|---|---|
| `aur/PKGBUILD` | Arch User Repository | `bash -n` syntax only |
| `homebrew/legatorocr.rb` | Homebrew tap | `ruby -c` syntax only |
| `winget/ChiutaAlexio.LegatorOCR.yaml` | winget | parsed as YAML only |
| `flatpak/tf.trom.LegatorOCR.yml` | Flathub | parsed as YAML only; needs vendored crates |
| `debian/control`, `debian/rules` | Debian | not built |
