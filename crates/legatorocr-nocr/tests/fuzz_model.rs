//! A damaged model file must be refused (or, when it still loads, run) without panicking.
use legatorocr_nocr::model::Model;

#[test]
fn damaged_model_files_never_panic() {
    let good = include_bytes!("../models/multi.lnm");
    let mut s: u64 = 0x2545F4914F6CDD1D;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    assert!(Model::from_bytes(good).is_ok());
    for it in 0..400 {
        let mut b = good.to_vec();
        match it % 4 {
            0 => b.truncate((rnd() as usize) % b.len()),
            1 => {
                // header and tensor directory live in the first bytes
                for _ in 0..4 {
                    let i = (rnd() as usize) % 200;
                    b[i] = rnd() as u8;
                }
            }
            2 => {
                let i = (rnd() as usize) % b.len();
                b[i] ^= 1 << (rnd() % 8);
            }
            _ => {
                let i = (rnd() as usize) % 64;
                b[i..i.saturating_add(4).min(64)].fill(0xFF);
            }
        }
        if let Ok(m) = Model::from_bytes(&b) {
            let w = 16 + (rnd() as usize) % 48;
            let img = vec![0.5f32; legatorocr_nocr::model::LINE_H * w];
            let _ = m.forward(&img, w);
        }
    }
}
