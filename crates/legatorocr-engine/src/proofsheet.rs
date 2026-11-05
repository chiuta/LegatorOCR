//! `--proof-sheet FILE.html` (with `--ocr`): one self-contained page for correcting the OCR by
//! hand. Each scanned page is shown next to an editable box holding the recognised text; words
//! the engine was unsure of (confidence under 70) are listed under the box. "Save corrected
//! text" downloads a plain-text file, pages separated by a form feed (U+000C). That file is a
//! hand-checked reference text (roadmap 0.1) once a person has read every page against its
//! picture. The program does not check the corrections.

use crate::report::base64;
use image::imageops::FilterType;
use legatorocr_core::{Page, Word};
use std::sync::Mutex;

struct PageRec {
    n: usize,
    jpeg: Vec<u8>,
    text: String,
    doubtful: Vec<String>,
}

static STATE: Mutex<Option<Vec<PageRec>>> = Mutex::new(None);

pub fn begin() {
    *STATE.lock().unwrap() = Some(Vec::new());
}

pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

/// Note one page (`n` counts from 1).
pub fn record(n: usize, pg: &Page, words: &[Word]) {
    let mut g = STATE.lock().unwrap();
    let Some(v) = g.as_mut() else { return };
    let img = match &pg.layers {
        Some(l) => image::DynamicImage::ImageRgb8(l.compose()),
        None => pg.image.to_dynamic(),
    };
    let img = if img.width() > 1100 {
        img.resize(1100, u32::MAX, FilterType::Triangle)
    } else {
        img
    };
    let mut jpeg = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 70);
    if img.to_rgb8().write_with_encoder(enc).is_err() {
        jpeg.clear();
    }
    let doubtful = words
        .iter()
        .filter(|w| w.conf >= 0.0 && w.conf < 70.0 && w.text.chars().any(|c| c.is_alphanumeric()))
        .map(|w| w.text.trim().to_string())
        .collect();
    v.push(PageRec {
        n,
        jpeg,
        text: legatorocr_ocr::words_to_text(words),
        doubtful,
    });
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The finished page, or `None` when nothing was recorded.
pub fn finish(title: &str) -> Option<String> {
    let pages = STATE.lock().unwrap().take()?;
    if pages.is_empty() {
        return None;
    }
    Some(render(&pages, title))
}

fn render(pages: &[PageRec], title: &str) -> String {
    let mut s = String::new();
    s.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    s.push_str(&format!("<title>{}</title><style>", esc(title)));
    s.push_str("body{font:15px system-ui,sans-serif;margin:0;background:#f4f4f2;color:#222}header{position:sticky;top:0;background:#fff;border-bottom:1px solid #ccc;padding:8px 16px;display:flex;gap:12px;align-items:center;z-index:2}section{display:flex;gap:16px;padding:16px;border-bottom:1px solid #ddd;flex-wrap:wrap}section>div{flex:1 1 420px;min-width:0}img{max-width:100%;border:1px solid #bbb;background:#fff}textarea{width:100%;min-height:60vh;font:15px/1.45 Georgia,serif;box-sizing:border-box;padding:8px}.q{font-size:13px;color:#8a4b00}h2{margin:0 0 6px;font-size:15px}button{font:inherit;padding:6px 12px}@media(prefers-color-scheme:dark){body{background:#1c1c1c;color:#ddd}header{background:#262626;border-color:#444}textarea{background:#222;color:#ddd}.q{color:#e0a050}}");
    s.push_str("</style></head><body><header><b>");
    s.push_str(&esc(title));
    s.push_str("</b><span id=\"c\"></span><button onclick=\"save()\">Save corrected text</button><span>Read each page against its picture and fix the text; pages are separated by a form feed in the saved file.</span></header>");
    for p in pages {
        s.push_str(&format!("<section><div><h2>Page {}</h2>", p.n));
        if p.jpeg.is_empty() {
            s.push_str("<p>(no picture)</p>");
        } else {
            s.push_str(&format!(
                "<img alt=\"page {}\" src=\"data:image/jpeg;base64,{}\">",
                p.n,
                base64(&p.jpeg)
            ));
        }
        s.push_str("</div><div><button onclick=\"sp(this)\" title=\"citește pagina cu vocea browserului\">🔊 Citește</button> <button onclick=\"speechSynthesis.cancel()\">⏹</button><textarea class=\"pg\" spellcheck=\"false\">");
        s.push_str(&esc(&p.text));
        s.push_str("</textarea>");
        if !p.doubtful.is_empty() {
            let mut d = p.doubtful.clone();
            d.truncate(60);
            s.push_str(&format!(
                "<p class=\"q\">Unsure: {}</p>",
                esc(&d.join(" · "))
            ));
        }
        s.push_str("</div></section>");
    }
    s.push_str("<script>function sp(b){try{speechSynthesis.cancel();const t=b.parentNode.querySelector('textarea').value.split(/(?<=[.!?\\n])\\s+/);t.forEach(x=>{if(x.trim()){const u=new SpeechSynthesisUtterance(x);u.lang='ro-RO';speechSynthesis.speak(u)}})}catch(e){}}const T=[...document.querySelectorAll('textarea.pg')];function n(){document.getElementById('c').textContent=T.length+' pages'}n();function save(){const b=new Blob([T.map(t=>t.value.replace(/\\s+$/,'')).join('\\n\\f\\n')+'\\n'],{type:'text/plain;charset=utf-8'});const a=document.createElement('a');a.href=URL.createObjectURL(b);a.download='reference.txt';a.click()}</script></body></html>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_is_self_contained_and_escaped() {
        let p = PageRec {
            n: 3,
            jpeg: vec![1, 2, 3],
            text: "A & <b>".into(),
            doubtful: vec!["x\"y".into()],
        };
        let h = render(&[p], "T<1>");
        assert!(
            h.contains("A &amp; &lt;b&gt;")
                && h.contains("Page 3")
                && h.contains("data:image/jpeg;base64,AQID")
        );
        assert!(h.contains("x&quot;y") && h.contains("T&lt;1&gt;") && !h.contains("<b>A"));
        assert!(!h.contains("http://") && !h.contains("src=\"http"));
    }
}
