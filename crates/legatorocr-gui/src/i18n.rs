//! Minimal i18n: a static table (key, Romanian, English). Missing keys show the key itself.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    Ro,
    En,
}

impl Lang {
    /// System language → Romanian if it starts with "ro", otherwise English.
    pub fn detect() -> Self {
        match sys_locale::get_locale() {
            Some(l) if l.to_ascii_lowercase().starts_with("ro") => Lang::Ro,
            _ => Lang::En,
        }
    }
}

pub fn t(lang: Lang, key: &'static str) -> &'static str {
    TABLE
        .iter()
        .find(|(k, _, _)| *k == key)
        .map(|(_, ro, en)| if lang == Lang::Ro { *ro } else { *en })
        .unwrap_or(key)
}

const TABLE: &[(&str, &str, &str)] = &[
    ("menu.file", "Fișier", "File"),
    ("menu.add_images", "Adaugă imagini…", "Add images…"),
    ("menu.add_folder", "Adaugă un folder…", "Add folder…"),
    ("menu.export_pdf", "Exportă PDF…", "Export PDF…"),
    ("menu.export_images", "Exportă imaginile…", "Export images…"),
    ("menu.quit", "Ieșire", "Quit"),
    ("menu.language", "Limbă", "Language"),
    ("menu.about", "Despre", "About"),
    ("about.text", "Post-procesare de scanări: separare pagini, îndreptare, decupare, curățare, PDF cu OCR. Implementare clean-room în Rust.", "Scan post-processing: page split, deskew, crop, clean-up, PDF with OCR. Clean-room implementation in Rust."),
    ("btn.add", "Adaugă", "Add"),
    ("btn.remove", "Elimină", "Remove"),
    ("btn.clear", "Golește", "Clear"),
    ("btn.up", "Sus", "Up"),
    ("btn.down", "Jos", "Down"),
    ("btn.cancel", "Anulează", "Cancel"),
    ("list.title", "Pagini", "Pages"),
    ("list.empty", "Trage aici imagini sau un folder, ori apasă „Adaugă”.", "Drop images or a folder here, or press “Add”."),
    ("drop.hint", "Eliberează pentru a adăuga", "Release to add"),
    ("preview.empty", "Adaugă scanări pentru a începe", "Add scans to get started"),
    ("preview.blank", "pagină goală", "blank page"),
    ("preview.skew", "înclinare corectată", "skew corrected"),
    ("view.both", "Alăturat", "Side by side"),
    ("view.original", "Original", "Original"),
    ("view.processed", "Procesat", "Processed"),
    ("view.fit", "Potrivește", "Fit"),
    ("adj.title", "Această pagină:", "This page:"),
    ("adj.rot_left", "Rotește la stânga 90°", "Rotate left 90°"),
    ("adj.rot_right", "Rotește la dreapta 90°", "Rotate right 90°"),
    ("adj.split_default", "implicit", "default"),
    ("adj.skew_manual", "Înclinare manuală", "Manual skew"),
    ("adj.reset", "Resetează", "Reset"),
    ("set.title", "Setări", "Settings"),
    ("set.input", "Intrare", "Input"),
    ("set.dpi", "Rezoluție (dpi)", "Resolution (dpi)"),
    ("set.auto_orient", "Detectează pagina culcată", "Detect sideways pages"),
    ("set.split", "Separare pagini", "Page split"),
    ("split.auto", "Automat", "Automatic"),
    ("split.middle", "La mijloc", "At the middle"),
    ("split.off", "Oprită", "Off"),
    ("set.geometry", "Geometrie", "Geometry"),
    ("set.deskew", "Îndreaptă automat", "Auto-deskew"),
    ("set.max_skew", "Înclinare maximă", "Max skew"),
    ("set.margin", "Margini", "Margins"),
    ("set.page_size", "Dimensiune pagină", "Page size"),
    ("size.free", "Liberă", "Free"),
    ("size.custom", "Personalizată", "Custom"),
    ("set.output", "Ieșire", "Output"),
    ("mode.color", "Color", "Colour"),
    ("mode.gray", "Gri", "Gray"),
    ("mode.bw", "Alb-negru", "Black & white"),
    ("set.sauvola", "Sensibilitate", "Sensitivity"),
    ("set.despeckle", "Elimină petele mici", "Remove small specks"),
    ("set.pdf", "PDF", "PDF"),
    ("set.quality", "Calitate JPEG", "JPEG quality"),
    ("set.quality_hint", "0 = fără pierderi. Paginile alb-negru sunt mereu fără pierderi.", "0 = lossless. Black & white pages are always lossless."),
    ("set.skip_blank", "Omite paginile goale", "Skip blank pages"),
    ("set.ocr", "OCR", "OCR"),
    ("set.ocr_enable", "Text căutabil (OCR) în PDF", "Searchable text (OCR) in PDF"),
    ("set.ocr_missing", "Tesseract nu a fost găsit. Instalează-l (și pachetul de limbă dorit) pentru OCR.", "Tesseract was not found. Install it (and the language pack you need) to enable OCR."),
    ("set.ocr_langs", "Limbi recunoscute", "Recognition languages"),
    ("set.psm", "Mod de segmentare", "Segmentation mode"),
    ("status.hint", "Pregătit. Alege o pagină pentru previzualizare.", "Ready. Select a page to preview."),
    ("status.added", "fișier(e) adăugate", "file(s) added"),
    ("status.working", "Se procesează…", "Working…"),
    ("status.pdf_saved", "PDF salvat", "PDF saved"),
    ("status.images_saved", "Imagini salvate în", "Images saved to"),
    ("status.pages", "pagini", "pages"),
    ("status.words", "cuvinte recunoscute", "words recognised"),
    ("status.blank_skipped", "pagini goale omise", "blank pages skipped"),
    ("status.failed", "fișiere eșuate (primul)", "file(s) failed (first)"),
    ("status.crashed", "procesarea s-a oprit neașteptat", "processing stopped unexpectedly"),
    ("status.cancelled", "Anulat.", "Cancelled."),
    ("status.error", "Eroare", "Error"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn table_is_complete_and_unique() {
        let mut seen = HashSet::new();
        for (k, ro, en) in TABLE {
            assert!(seen.insert(*k), "duplicate key {k}");
            assert!(!ro.is_empty() && !en.is_empty(), "empty text for {k}");
        }
    }

    #[test]
    fn every_key_used_in_the_app_exists() {
        const GROUPS: &[&str] = &["about", "adj", "btn", "drop", "list", "menu", "mode", "preview", "set", "size", "split", "status", "view"];
        let src = include_str!("app.rs");
        let mut checked = 0;
        for lit in src.split('"').skip(1).step_by(2) {
            let Some((g, rest)) = lit.split_once('.') else { continue };
            if GROUPS.contains(&g) && !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
                assert!(TABLE.iter().any(|(k, _, _)| *k == lit), "app.rs uses unknown i18n key {lit}");
                checked += 1;
            }
        }
        assert!(checked > 50, "only {checked} keys found — scanner broken?");
    }

    #[test]
    fn lookup_and_fallback() {
        assert_eq!(t(Lang::Ro, "menu.file"), "Fișier");
        assert_eq!(t(Lang::En, "menu.file"), "File");
        assert_eq!(t(Lang::En, "no.such.key"), "no.such.key");
    }
}
