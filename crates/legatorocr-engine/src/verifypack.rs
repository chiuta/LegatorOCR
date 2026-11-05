//! `--verify-pack FILE.html` (with `--ocr`): the words the engine was unsure of, one card each, in
//! one self-contained page that works offline in any browser. A person sees the word picture and
//! the recognised text; **Enter** confirms (or saves what was typed), **→** skips, **←** goes
//! back, **R** reads the word aloud with the browser's own voice (if it has a Romanian one; this
//! is the browser's function, not checked here). The result is exported as JSON with the word
//! pictures, which `tools/ocr-train/import_verify.py` turns into labelled training lines. The
//! program does not check the answers: they are what the person typed. Confirmed words are
//! "unsure words that were right", so a model trained on them alone is biased towards the
//! engine's own reading; the corrected ones are the valuable part.

use crate::report::base64;
use image::imageops::FilterType;
use legatorocr_core::{Page, Word};
use std::sync::Mutex;

struct Item {
    page: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    text: String,
    conf: f32,
    jpeg: Vec<u8>,
}

struct State {
    conf_max: f32,
    per_page: usize,
    items: Vec<Item>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// `conf_max`: words under this confidence get a card; `per_page`: at most this many per page
/// (the least confident first).
pub fn begin(conf_max: f32, per_page: usize) {
    *STATE.lock().unwrap() = Some(State {
        conf_max,
        per_page: per_page.max(1),
        items: Vec::new(),
    });
}

pub fn enabled() -> bool {
    STATE.lock().unwrap().is_some()
}

pub fn record(n: usize, pg: &Page, words: &[Word]) {
    let mut g = STATE.lock().unwrap();
    let Some(st) = g.as_mut() else { return };
    let mut cand: Vec<&Word> = words
        .iter()
        .filter(|w| {
            w.conf >= 0.0
                && w.conf < st.conf_max
                && w.w >= 6
                && w.h >= 6
                && w.text.chars().any(|c| c.is_alphanumeric())
        })
        .collect();
    if cand.is_empty() {
        return;
    }
    cand.sort_by(|a, b| {
        a.conf
            .partial_cmp(&b.conf)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    cand.truncate(st.per_page);
    cand.sort_by_key(|w| (w.line, w.x));
    let img = pg.image.to_dynamic().to_luma8();
    for w in cand {
        let pad = 3u32;
        let x0 = w.x.saturating_sub(pad);
        let y0 = w.y.saturating_sub(pad);
        let x1 = (w.x + w.w + pad).min(img.width());
        let y1 = (w.y + w.h + pad).min(img.height());
        if x1 <= x0 + 2 || y1 <= y0 + 2 {
            continue;
        }
        let crop = image::imageops::crop_imm(&img, x0, y0, x1 - x0, y1 - y0).to_image();
        let crop = if crop.height() > 96 {
            let nw = (crop.width() as f32 * 96.0 / crop.height() as f32).max(1.0) as u32;
            image::imageops::resize(&crop, nw, 96, FilterType::Triangle)
        } else {
            crop
        };
        let mut jpeg = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85);
        if crop.write_with_encoder(enc).is_err() {
            continue;
        }
        st.items.push(Item {
            page: n,
            x: w.x,
            y: w.y,
            w: w.w,
            h: w.h,
            text: w.text.trim().to_string(),
            conf: w.conf,
            jpeg,
        });
    }
}

/// JSON string literal that is safe inside a `<script>` element.
fn js(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            '\r' => o.push_str("\\r"),
            '<' => o.push_str("\\u003c"),
            '>' => o.push_str("\\u003e"),
            '&' => o.push_str("\\u0026"),
            '\u{2028}' => o.push_str("\\u2028"),
            '\u{2029}' => o.push_str("\\u2029"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The finished page, or `None` when no word needed a card.
pub fn finish(title: &str) -> Option<String> {
    let st = STATE.lock().unwrap().take()?;
    if st.items.is_empty() {
        return None;
    }
    Some(render(&st.items, title))
}

fn render(items: &[Item], title: &str) -> String {
    let mut data = String::from("[");
    for (i, it) in items.iter().enumerate() {
        if i > 0 {
            data.push(',');
        }
        data.push_str(&format!(
            "{{\"p\":{},\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"o\":{},\"c\":{:.0},\"j\":\"{}\"}}",
            it.page,
            it.x,
            it.y,
            it.w,
            it.h,
            js(&it.text),
            it.conf,
            base64(&it.jpeg)
        ));
    }
    data.push(']');
    let mut s = String::new();
    s.push_str("<!doctype html><html lang=\"ro\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    s.push_str(&format!("<title>{}</title><style>", esc(title)));
    s.push_str(":root{--bg:#f4f4f2;--fg:#222;--card:#fff;--line:#ccc;--ok:#1b7a3a;--ac:#2a5db0}@media(prefers-color-scheme:dark){:root{--bg:#1c1c1c;--fg:#ddd;--card:#262626;--line:#444;--ok:#5fcf80;--ac:#7fa8ff}}body{font:16px system-ui,sans-serif;margin:0;background:var(--bg);color:var(--fg)}header{padding:10px 16px;border-bottom:1px solid var(--line);background:var(--card);display:flex;gap:12px;align-items:center;flex-wrap:wrap}main{max-width:720px;margin:0 auto;padding:16px}.card{background:var(--card);border:1px solid var(--line);border-radius:10px;padding:16px;text-align:center}.card img{max-width:100%;image-rendering:auto;background:#fff;border:1px solid var(--line);min-height:48px;max-width:100%;object-fit:contain}input{font:28px/1.2 Georgia,serif;width:100%;box-sizing:border-box;padding:8px;text-align:center;margin:12px 0}button{font:inherit;padding:8px 14px;margin:2px}.row{display:flex;gap:6px;justify-content:center;flex-wrap:wrap}progress{width:200px}small{opacity:.75}");
    s.push_str("</style></head><body><header><b>");
    s.push_str(&esc(title));
    s.push_str("</b><progress id=\"pr\"></progress><span id=\"st\"></span><button onclick=\"exp()\">Exportă răspunsurile</button><label><input type=\"file\" id=\"fi\" accept=\".json\" style=\"display:none\" onchange=\"imp(this)\"><button onclick=\"fi.click()\">Încarcă răspunsuri</button></label></header><main><div class=\"card\" id=\"cd\"><small id=\"mt\"></small><br><img id=\"im\" alt=\"cuvânt\"><input id=\"tx\" spellcheck=\"false\" autocomplete=\"off\"><div class=\"row\"><button onclick=\"go(-1)\">← Înapoi</button><button onclick=\"ok()\" style=\"color:var(--ok)\">✓ Enter: confirm</button><button onclick=\"go(1)\">Sari →</button><button onclick=\"say()\">🔊 R</button></div><p><small>Enter confirmă (sau salvează ce ai scris). → sari, ← înapoi, Ctrl+R citește cuvântul. Răspunsurile rămân în acest browser și se pot exporta.</small></p></div></main><script>const D=");
    s.push_str(&data);
    s.push_str(";const K='verify:'+D.length+':'+D[0].j.slice(0,40);let A={};try{A=JSON.parse(localStorage.getItem(K)||'{}')}catch(e){}let i=0;const $=id=>document.getElementById(id);function sv(){try{localStorage.setItem(K,JSON.stringify(A))}catch(e){}}function show(){const d=D[i];$('im').src='data:image/jpeg;base64,'+d.j;$('im').style.height=Math.min(140,Math.max(48,d.h*3))+'px';const a=A[i];$('tx').value=a?a.t:d.o;$('mt').textContent='pagina '+d.p+' · încredere motor '+d.c+' %'+(a?' · '+(a.t!==d.o?'corectat':'confirmat'):'');$('tx').focus();$('tx').select();const n=Object.keys(A).length;$('pr').max=D.length;$('pr').value=n;$('st').textContent=(i+1)+' / '+D.length+' · verificate '+n+' · corectate '+Object.values(A).filter((v,k)=>v.t!==v.o).length}function go(k){i=Math.min(D.length-1,Math.max(0,i+k));show()}function ok(){const d=D[i];A[i]={t:$('tx').value.trim(),o:d.o};sv();if(i<D.length-1)i++;show()}function say(){try{const u=new SpeechSynthesisUtterance($('tx').value);u.lang='ro-RO';speechSynthesis.cancel();speechSynthesis.speak(u)}catch(e){}}document.addEventListener('keydown',e=>{if(e.key==='Enter'){e.preventDefault();ok()}else if(e.key==='ArrowRight'){e.preventDefault();go(1)}else if(e.key==='ArrowLeft'){e.preventDefault();go(-1)}else if((e.key==='r'||e.key==='R')&&e.ctrlKey){e.preventDefault();say()}});function exp(){const o=Object.keys(A).map(k=>{const d=D[k];return{page:d.p,x:d.x,y:d.y,w:d.w,h:d.h,ocr:d.o,text:A[k].t,corrected:A[k].t!==d.o,jpeg:d.j}});const b=new Blob([JSON.stringify(o)],{type:'application/json'});const a=document.createElement('a');a.href=URL.createObjectURL(b);a.download='verified.json';a.click()}function imp(f){const r=new FileReader();r.onload=()=>{try{JSON.parse(r.result).forEach(v=>{const k=D.findIndex(d=>d.p===v.page&&d.x===v.x&&d.y===v.y);if(k>=0)A[k]={t:v.text,o:D[k].o}});sv();show()}catch(e){}};r.readAsText(f.files[0])}show();</script></body></html>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkpage(w: u32, h: u32) -> Page {
        Page {
            image: legatorocr_core::PageImage::Gray(image::GrayImage::from_pixel(
                w,
                h,
                image::Luma([255]),
            )),
            dpi: 150,
            skew_deg: 0.0,
            blank: false,
            split_index: None,
            content_box: None,
            layers: None,
        }
    }

    fn word(text: &str, conf: f32, x: u32) -> Word {
        Word {
            text: text.into(),
            x,
            y: 5,
            w: 30,
            h: 12,
            conf,
            line: 0,
        }
    }

    #[test]
    fn only_unsure_words_get_a_card_and_a_page_without_any_gives_nothing() {
        begin(70.0, 2);
        let pg = mkpage(200, 40);
        let words = vec![
            word("sigur", 95.0, 5),
            word("ab", 40.0, 50),
            word("cd", 60.0, 90),
            word("ef", 10.0, 130),
            word("1</script>", 20.0, 160),
        ];
        record(3, &pg, &words);
        let g = STATE.lock().unwrap();
        let st = g.as_ref().unwrap();
        assert_eq!(st.items.len(), 2, "at most per_page, least confident first");
        assert_eq!(
            st.items.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            ["ef", "1</script>"]
        );
        drop(g);
        let h = finish("T<1>").unwrap();
        assert!(
            h.contains("T&lt;1&gt;")
                && !h.contains("1</script>")
                && h.contains("1\\u003c/script\\u003e")
        );
        assert!(!h.contains("http://") && !h.contains("src=\"http"));
        assert!(!enabled());
        // (same test: the state is global, so tests that use it must not run in parallel)
        begin(70.0, 5);
        let pg = mkpage(50, 20);
        record(1, &pg, &[word("sigur", 99.0, 5)]);
        assert!(finish("t").is_none());
    }

    #[test]
    fn js_strings_cannot_close_the_script() {
        let s = js("a\"b\\c</script><!--\n\u{2028}");
        assert!(!s.contains('<') && !s.contains('\n') && !s.contains('\u{2028}'));
    }
}
