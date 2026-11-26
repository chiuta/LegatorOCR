//! Mutation fuzzing of the PDF reader: every public entry point on damaged copies of a real
//! file must return (Ok or Err), never panic or hang.
//! `LEGATOROCR_FUZZ_PDF=file.pdf LEGATOROCR_FUZZ_ITERS=n` use another seed document.
use legatorocr_pdf::reader::PdfDoc;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

const MINI: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj\n2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj\n3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R/Resources<</Font<</F1 5 0 R>>/XObject<</I 6 0 R>>>>>>endobj\n4 0 obj<</Length 60>>stream\nq 100 0 0 100 10 10 cm /I Do Q BT /F1 12 Tf 10 150 Td (Hello) Tj ET\nendstream endobj\n5 0 obj<</Type/Font/Subtype/Type1/BaseFont/Helvetica>>endobj\n6 0 obj<</Type/XObject/Subtype/Image/Width 2/Height 2/ColorSpace/DeviceGray/BitsPerComponent 8/Length 4>>stream\n\x00\x80\xff\x40\nendstream endobj\ntrailer<</Root 1 0 R/Size 7>>\n%%EOF\n";

fn exercise(data: Vec<u8>) {
    if let Ok(d) = PdfDoc::open(data) {
        let n = d.page_count().min(4);
        for i in 0..n {
            let _ = d.page_info(i);
            let _ = d.page_image(i);
            let _ = d.page_text(i);
        }
        let _ = d.info_text("Title");
        let _ = d.is_encrypted();
        let _ = d.decrypted();
        let _ = d.flattened();
        let _ = d.embedded_files();
        let _ = d.active_content();
    }
}

#[test]
fn damaged_pdfs_never_panic() {
    let seed_doc = std::env::var("LEGATOROCR_FUZZ_PDF")
        .ok()
        .and_then(|p| std::fs::read(p).ok())
        .unwrap_or_else(|| MINI.to_vec());
    let iters: usize = std::env::var("LEGATOROCR_FUZZ_ITERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(300);
    let mut r = Rng(0x9E3779B97F4A7C15);
    exercise(seed_doc.clone());
    for _ in 0..iters {
        let mut d = seed_doc.clone();
        for _ in 0..1 + r.below(6) {
            let i = r.below(d.len());
            match r.below(5) {
                0 => d[i] ^= 1 << r.below(8),
                1 => d[i] = r.next() as u8,
                2 => d.truncate(i.max(8)),
                3 => {
                    let j = r.below(d.len());
                    let k = (i + 1 + r.below(64)).min(d.len());
                    let chunk = d[i..k].to_vec();
                    let j = j.min(d.len());
                    d.splice(j..j, chunk);
                }
                _ => {
                    // numbers become huge: the classic dimension / count bomb
                    if let Some(p) = d[i..].iter().position(|b| b.is_ascii_digit()) {
                        let at = i + p;
                        d.splice(at..at, b"99999999999".iter().copied());
                    }
                }
            }
            if d.is_empty() {
                d.push(b'%');
            }
        }
        exercise(d);
    }
}
