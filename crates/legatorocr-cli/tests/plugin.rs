//! --plugin: a WebAssembly filter runs on every loaded page, and a broken one stops the run.

use std::process::Command;

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

fn run(d: &std::path::Path, out: &str, extra: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_legatorocr"))
        .current_dir(d)
        .args(["a.png", "--dpi", "300", "--mode", "gray", "-o", out])
        .args(extra)
        .output()
        .unwrap()
}

fn first_png(d: &std::path::Path) -> image::GrayImage {
    let f = std::fs::read_dir(d).unwrap().filter_map(|e| e.ok()).map(|e| e.path())
        .find(|p| p.extension().is_some_and(|e| e == "png")).expect("an output page");
    image::open(f).unwrap().to_luma8()
}

#[test]
fn a_wasm_plugin_changes_the_pages_and_a_broken_one_is_refused() {
    let d = std::env::temp_dir().join(format!("lgr_plugin_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let mut img = image::GrayImage::from_pixel(400, 300, image::Luma([255]));
    for y in 60..70 {
        for x in 40..300 {
            img.put_pixel(x, y, image::Luma([0]));
        }
    }
    img.save(d.join("a.png")).unwrap();
    std::fs::write(d.join("invert.wasm"), wat::parse_str(INVERT).unwrap()).unwrap();
    std::fs::write(d.join("bad.wasm"), b"not wasm").unwrap();

    let plain = run(&d, "plain", &[]);
    assert!(plain.status.success(), "{}", String::from_utf8_lossy(&plain.stderr));
    let inv = run(&d, "inv", &["--plugin", "invert.wasm"]);
    assert!(inv.status.success(), "{}", String::from_utf8_lossy(&inv.stderr));
    let mean = |g: &image::GrayImage| g.pixels().map(|p| p[0] as u64).sum::<u64>() / g.pixels().len() as u64;
    let (m0, m1) = (mean(&first_png(&d.join("plain"))), mean(&first_png(&d.join("inv"))));
    assert!(m0 > 200 && m1 + 50 < m0, "plain page mean {m0}, inverted page mean {m1}");

    let bad = run(&d, "bad", &["--plugin", "bad.wasm"]);
    assert!(!bad.status.success());
    assert!(String::from_utf8_lossy(&bad.stderr).contains("WebAssembly"), "{}", String::from_utf8_lossy(&bad.stderr));
    let _ = std::fs::remove_dir_all(&d);
}
