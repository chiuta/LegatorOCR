# Audit: Cargo dependencies, advisories, toolchain (read 2026-11-04)

Scope: `Cargo.lock` of the workspace (354 packages = 8 own crates + 346 from crates.io). Everything below was read
from the live sources on 2026-11-04; nothing is from memory.

## 1. Newest version on crates.io

Source: sparse index `https://index.crates.io/<path>` (one request per crate name, 322 names; yanked and
pre-release versions ignored). `cargo update --dry-run` was used as the authority for compatible bumps.

| Direct dependency | In repo (lock) | Newest | Verdict |
|---|---|---|---|
| image | 0.25.10 | 0.25.10 | current |
| flate2 | 1.1.10 | 1.1.10 | current |
| clap | 4.6.7 | 4.6.7 | current |
| anyhow | 1.0.104 | 1.0.104 | current |
| thiserror | 2.x | 2.0.21 | current (a 1.0.69 copy is pulled in by another crate) |
| rayon / regex | 1.12.0 / 1.13.1 | same | current |
| eframe | 0.37.0 | 0.37.0 | current |
| memmap2 / rfd / rqrr / sys-locale / tiff | 0.9.12 / 0.17.2 / 0.11.0 / 0.3.2 / 0.11.3 | same | current |
| wat / djvu-rs / wasmi | 1.263.0 / 0.44.0 / 2.0.0 | same | current |
| **getrandom** | 0.3.4 | **0.4.3** | behind, 0.x minor = breaking; NOT done |

Whole lock (346 crates.io packages), before the update: 90 package-versions were behind the newest release
(3 of them are index artefacts for `toml_*`, whose versions carry `+spec` build metadata; I excluded those, leaving 87:
2 patch, 71 minor, 14 major; for `0.x` crates a "minor" step is a breaking step in semver).
Of these 3 were compatible bumps (see section 3); **84 remain behind**: 14 major + 70 minor.
All 84 are transitive (except `getrandom`) and held back by the `eframe`/`winit`/`wgpu`/`image`/`wasmi` stack, which
is current itself; many are older duplicates of a version that is also in the tree (e.g. `bitflags` 1.3.2 next to
2.13.2, `block2` 0.5.1 next to 0.6.2). They cannot be bumped from this repo without patching other crates.

Major steps behind (transitive): bitflags 1.3.2 -> 2.13.2, downcast-rs 1.2.1 -> 2.0.2, fearless_simd 0.4.1 -> 1.2.0,
pollster 0.4.0 -> 1.0.1, r-efi 5.3.0/6.0.0 -> 7.1.0, rustc-hash 1.1.0 -> 2.1.3, rustix 0.38.44 -> 1.1.6,
safe_arch 0.7.4 -> 1.2.0, syn 2.0.119 -> 3.0.9, thiserror 1.0.69 -> 2.0.21, wide 0.7.33 -> 1.7.1, xml-rs 0.8.29 -> 1.0.0.

## 2. RustSec advisories

Source: `https://github.com/rustsec/advisory-db` (shallow clone of `main`, commit `965bf9ad083b`, newest advisory
date in it 2026-11-03; raw.githubusercontent.com has no directory listing, so the whole tree was cloned instead).
Method: for every `crates/<name>/RUSTSEC-*.md` of a crate that is in the lock, read `patched` / `unaffected` from the TOML
header and test the locked version against them with a small hand-written semver matcher (checked on a known case:
`time 0.1.40` is reported affected by RUSTSEC-2020-0071).

| Result | Count |
|---|---|
| Advisories that exist for crates in the lock (by name) | 48 (27 crates) |
| ... of which affect the locked version | **0** |
| ... of which the locked version is patched/unaffected | 46 |
| ... withdrawn (spin RUSTSEC-2019-0031, xml-rs RUSTSEC-2022-0048; both "unmaintained" notices) | 2 |

Crates with (all inapplicable) advisories: anstream, anyhow, bumpalo, bytes, crossbeam-*, futures-*, hashbrown, image,
lock_api, lru, memmap2, once_cell, quick-xml, regex, self_cell, shlex, slab, smallvec, spin, string-interner, tokio,
tracing, xml-rs, zerocopy, zlib-rs.
Not verified: `[affected]` platform/function narrowing in advisories was not evaluated (it only narrows further, so
no hit can be hidden by it); the nightly `audit.yml` job (`cargo audit --deny warnings`) is the independent check
and I could not run `cargo-audit` here (not installed, installing it needs crates.io builds I was told to avoid).

## 3. Update and gate

`cargo update` (compatible bumps only), 4 packages, no code change:

| Crate | From | To | Licence |
|---|---|---|---|
| cc | 1.10.0 | 1.11.0 | MIT OR Apache-2.0 (same) |
| crc32fast | 1.5.3 | 1.5.4 | MIT OR Apache-2.0 (same) |
| toml_edit | 0.25.17 | 0.25.18 | MIT OR Apache-2.0 (same) |
| unicode-properties | 0.1.4 | 0.1.5 | **changed**: `MIT/Apache-2.0` -> `(MIT OR Apache-2.0) AND Unicode-3.0` (Unicode data files added; permissive, notice in the crate's `LICENSE-UNICODE`) |

No advisory-driven bump was needed (0 hits). Not done: every major/breaking step in section 1 (`getrandom` 0.3 -> 0.4
is the only one in our own manifests; not needed for any advisory, not tried).

| Gate (CARGO_TARGET_DIR=target-wave2, debug) | Before | After |
|---|---|---|
| `cargo build --workspace` | not run separately | ok, 0 warnings |
| `cargo test --workspace --no-fail-fast` | 518 passed, 0 failed, 6 ignored | 518 passed, 0 failed, 6 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | not run | clean |

No bump had to be reverted. Test counts are equal because the bumps are patch-level in behaviour; this shows the bumps
do not break the suite, not that they fix anything.

## 4. Toolchain / MSRV

| Item | Repo says | Source says (2026-11-04) |
|---|---|---|
| `rust-version` | 1.95 (`Cargo.toml`, all 8 crates inherit) | 1.95.0 released 2026-04-16 |
| CI `msrv` job | toolchain `"1.95"`, `cargo check --locked --workspace` | same |
| Other CI jobs | `stable` (rolling) | latest stable = 1.99.1 (2026-10-15); 1.99.0 on 2026-10-01 |
| Local toolchain used here | n/a | rustc 1.97.0 |

Source for releases: `RELEASES.md` of `rust-lang/rust` (raw.githubusercontent.com). `static.rust-lang.org` is blocked
from this shell, so the stable manifest was not read.
MSRV evidence: the highest `rust-version` declared by any locked crate is **1.95** (`eframe`/`egui`/`egui-wgpu`/
`egui-winit`/`egui_glow`/`emath`/`epaint` 0.37.0, from the index), so 1.95 is the lowest value that dependencies allow;
the 4 bumped crates declare <= 1.65. **Not verified:** that the workspace actually compiles on 1.95.0 (no 1.95 toolchain
installed, download blocked); only the CI `msrv` job can prove it. 1.95 is a conservative floor, not stale: it is the
declared minimum of the GUI stack, and no change was made.

## 5. THIRD_PARTY_LICENSES.md

Each licence was compared with the `license` field of the crate's `Cargo.toml` in the registry source
(`~/.cargo/registry/src`). Result before: the 318 listed rows matched, but **28 crates of the lock were missing** from
the table (djvu-rs and its 7 sub-crates, wasmi + 3, wat/wast/wasm-encoder/2 wasmparser, rqrr, g2gen/g2p/g2poly, leb128fmt,
lru, png 0.17, safe_arch, wide, spin, string-interner). Now: 346 rows, 0 mismatches (script comparison). Rows added and
the 4 updated versions above. Worth knowing: `rqrr` is `(MIT OR Apache-2.0) AND ISC` (both apply, not a choice).
No licence was changed in the project itself.
