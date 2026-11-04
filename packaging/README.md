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
| `debian/control`, `debian/rules` | Debian source skeleton | not built, not submitted |
| `debian/build-deb.sh` | Debian/Ubuntu binary .deb | built with `dpkg-deb` from the release binary; installed, run (`--version`) and removed with `dpkg` in a container (amd64 only; no lintian, no other distro) |

## Portable folder (roadmap 1.11)

`portableapps/build.py BIN_DIR OUT_DIR VERSION` builds a PortableApps-layout folder and zip
(`App/AppInfo/appinfo.ini`, `App/LegatorOCR/`, `Data/`, `Other/Source/`). A `portable.txt` next
to the programs puts the GUI settings in `Data` instead of the user's profile (unit-tested in
`legatorocr-gui`; layout built by the Windows release job). Not done: the PortableApps.com
Launcher `.exe`, icons and the installer (Windows tools), and no run of the result on a
Windows PC; the folder starts the GUI through `Start-LegatorOCR.cmd`.
