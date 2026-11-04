//! WASM plugins (roadmap 7.8): an image filter in a WebAssembly module, run in the
//! `wasmi` interpreter with no imports (no files, no network, no clock), a fuel limit
//! and a memory cap, so a plugin can neither escape nor hang the run.
//!
//! ABI (all i32): the module exports `memory`, `lgr_alloc(len) -> ptr` and
//! `lgr_filter(ptr, width, height) -> status`. The host copies the page as packed
//! RGB8 into `ptr`, calls `lgr_filter`, and reads the buffer back; status 0 = ok.
use anyhow::{anyhow, bail, Context, Result};
use image::{DynamicImage, RgbImage};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use wasmi::{Config, Engine, Linker, Module, Store};

/// Instructions a plugin may execute per page before it is stopped.
const FUEL: u64 = 20_000_000_000;
/// Largest linear memory a plugin may use.
const MAX_MEMORY: usize = 2 << 30;

static PLUGIN: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Apply `path` to every page loaded from now on (None switches it off).
pub fn set(path: Option<&Path>) {
    *PLUGIN.lock().unwrap() = path.map(|p| p.to_path_buf());
}

/// The configured plugin, if any.
pub fn current() -> Option<PathBuf> {
    PLUGIN.lock().unwrap().clone()
}

/// Run the plugin at `path` over `img`.
pub fn run(path: &Path, img: &DynamicImage) -> Result<DynamicImage> {
    let bytes = std::fs::read(path).with_context(|| format!("plugin {}", path.display()))?;
    run_bytes(&bytes, img)
}

pub fn run_bytes(wasm: &[u8], img: &DynamicImage) -> Result<DynamicImage> {
    run_with_fuel(wasm, img, FUEL)
}

pub fn run_with_fuel(wasm: &[u8], img: &DynamicImage, fuel: u64) -> Result<DynamicImage> {
    let rgb = img.to_rgb8();
    let (w, h) = rgb.dimensions();
    let len = rgb.as_raw().len();
    if len > MAX_MEMORY / 2 {
        bail!("page too large for a plugin");
    }
    let mut config = Config::default();
    config.consume_fuel(true);
    let engine = Engine::new(&config);
    let module =
        Module::new(&engine, wasm).map_err(|e| anyhow!("plugin is not valid WebAssembly: {e}"))?;
    if module.imports().next().is_some() {
        bail!("plugin imports functions; plugins get no host functions");
    }
    let mut store = Store::new(&engine, ());
    store.set_fuel(fuel).map_err(|e| anyhow!("{e}"))?;
    let linker = <Linker<()>>::new(&engine);
    let inst = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(|e| anyhow!("plugin start: {e}"))?;
    let mem = inst
        .get_memory(&store, "memory")
        .ok_or_else(|| anyhow!("plugin exports no `memory`"))?;
    let alloc = inst
        .get_typed_func::<i32, i32>(&store, "lgr_alloc")
        .map_err(|e| anyhow!("plugin lacks lgr_alloc(i32)->i32: {e}"))?;
    let filter = inst
        .get_typed_func::<(i32, i32, i32), i32>(&store, "lgr_filter")
        .map_err(|e| anyhow!("plugin lacks lgr_filter(i32,i32,i32)->i32: {e}"))?;
    let ptr = alloc
        .call(&mut store, len as i32)
        .map_err(|e| anyhow!("lgr_alloc: {e}"))? as usize;
    if ptr
        .checked_add(len)
        .is_none_or(|e| e > mem.data_size(&store))
    {
        bail!("plugin allocated outside its memory");
    }
    mem.data_mut(&mut store)[ptr..ptr + len].copy_from_slice(rgb.as_raw());
    let status = filter
        .call(&mut store, (ptr as i32, w as i32, h as i32))
        .map_err(|e| anyhow!("lgr_filter stopped: {e}"))?;
    if status != 0 {
        bail!("plugin reported status {status}");
    }
    let out = mem.data(&store)[ptr..ptr + len].to_vec();
    Ok(DynamicImage::ImageRgb8(
        RgbImage::from_raw(w, h, out).ok_or_else(|| anyhow!("bad plugin output"))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Inverts every byte. Written by hand in WAT so the test needs no wasm toolchain.
    const INVERT: &str = r#"(module
      (memory (export "memory") 1)
      (global $top (mut i32) (i32.const 16))
      (func (export "lgr_alloc") (param $n i32) (result i32)
        (local $p i32)
        (local.set $p (global.get $top))
        (global.set $top (i32.add (global.get $top) (local.get $n)))
        (if (i32.gt_u (global.get $top) (i32.mul (memory.size) (i32.const 65536)))
          (then (drop (memory.grow (i32.add (i32.div_u (local.get $n) (i32.const 65536)) (i32.const 1))))))
        (local.get $p))
      (func (export "lgr_filter") (param $p i32) (param $w i32) (param $h i32) (result i32)
        (local $i i32) (local $n i32)
        (local.set $n (i32.mul (i32.mul (local.get $w) (local.get $h)) (i32.const 3)))
        (block $done (loop $l
          (br_if $done (i32.ge_u (local.get $i) (local.get $n)))
          (i32.store8 (i32.add (local.get $p) (local.get $i))
            (i32.xor (i32.load8_u (i32.add (local.get $p) (local.get $i))) (i32.const 255)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $l)))
        (i32.const 0)))"#;

    fn img() -> DynamicImage {
        DynamicImage::ImageRgb8(RgbImage::from_fn(5, 4, |x, y| {
            image::Rgb([x as u8 * 10, y as u8 * 20, 7])
        }))
    }

    #[test]
    fn a_plugin_filters_the_page() {
        let wasm = wat::parse_str(INVERT).unwrap();
        let out = run_bytes(&wasm, &img()).unwrap().to_rgb8();
        let src = img().to_rgb8();
        for (a, b) in src.pixels().zip(out.pixels()) {
            for c in 0..3 {
                assert_eq!(a[c] ^ 255, b[c]);
            }
        }
    }

    #[test]
    fn a_plugin_that_loops_forever_is_stopped_and_one_with_imports_is_refused() {
        let spin = wat::parse_str(
            r#"(module (memory (export "memory") 1)
              (func (export "lgr_alloc") (param i32) (result i32) (i32.const 16))
              (func (export "lgr_filter") (param i32 i32 i32) (result i32) (loop $l (br $l)) (i32.const 0)))"#,
        )
        .unwrap();
        assert!(run_with_fuel(&spin, &img(), 1_000_000).is_err());
        let imp =
            wat::parse_str(r#"(module (import "env" "x" (func)) (memory (export "memory") 1))"#)
                .unwrap();
        let e = run_bytes(&imp, &img()).unwrap_err().to_string();
        assert!(e.contains("imports"), "{e}");
    }
}
