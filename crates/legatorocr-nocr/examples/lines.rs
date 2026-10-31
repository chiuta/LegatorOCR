//! Developer check: run the recogniser on already-normalised 32-pixel line PNGs
//! (`MODEL DIR N`) and print `file<TAB>text`, for comparison with the training framework.
use legatorocr_nocr::{ctc, Model, LINE_H};

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let m = Model::from_bytes(&std::fs::read(&a[1]).unwrap()).unwrap();
    let n: usize = a[3].parse().unwrap();
    let t0 = std::time::Instant::now();
    for i in 0..n {
        let g = image::open(format!("{}/l{i}.png", a[2])).unwrap().to_luma8();
        assert_eq!(g.height() as usize, LINE_H);
        let w = g.width() as usize;
        let data: Vec<f32> = g.pixels().map(|p| (255 - p.0[0]) as f32 / 255.0).collect();
        let (t, lp) = m.forward(&data, w);
        let s: String = ctc::greedy(&lp, t, m.nc, &m.charset).iter().map(|g| g.ch).collect();
        println!("l{i}.png\t{s}");
    }
    eprintln!("{:?} per line", t0.elapsed() / n as u32);
}
