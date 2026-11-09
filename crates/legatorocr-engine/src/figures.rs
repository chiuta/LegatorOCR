//! Figures with alternative text for tagged PDF (`--figure`). A figure is a rectangle of a page that
//! the person names and describes: the program does not look at the picture and does not invent
//! the description. PDF/A level A asks for alternative text on every figure; here the figures
//! are the ones the person lists.

use anyhow::{bail, Result};
use std::sync::Mutex;

/// One figure: page (1-based, in the whole PDF), box in mm from the top left corner, description.
#[derive(Debug, Clone, PartialEq)]
pub struct Spec {
    pub page: usize,
    pub rect_mm: [f32; 4],
    pub alt: String,
}

static CFG: Mutex<Vec<Spec>> = Mutex::new(Vec::new());
/// Pages that `for_page` was asked about.
static ASKED: Mutex<Vec<usize>> = Mutex::new(Vec::new());

/// `PAGE,X,Y,W,H=DESCRIPTION` (mm from the top left of the page).
pub fn parse(s: &str) -> Result<Spec> {
    let Some((geo, alt)) = s.split_once('=') else {
        bail!("{s:?}: expected PAGE,X,Y,W,H=DESCRIPTION");
    };
    let alt = alt.trim();
    if alt.is_empty() {
        bail!("{s:?}: the description (alternative text) must not be empty");
    }
    let v: Vec<f32> = geo
        .split(',')
        .map(|x| x.trim().parse::<f32>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("{s:?}: expected PAGE,X,Y,W,H as numbers"))?;
    if v.len() != 5
        || v.iter().any(|x| !x.is_finite() || *x < 0.0)
        || v[0] < 1.0
        || v[0].fract() != 0.0
        || v[3] <= 0.0
        || v[4] <= 0.0
    {
        bail!("{s:?}: expected a page number from 1 and X,Y,W,H in mm (width and height above 0)");
    }
    Ok(Spec {
        page: v[0] as usize,
        rect_mm: [v[1], v[2], v[3], v[4]],
        alt: alt.to_string(),
    })
}

pub fn set(specs: Vec<Spec>) {
    if let Ok(mut a) = ASKED.lock() {
        a.clear();
    }
    if let Ok(mut g) = CFG.lock() {
        *g = specs;
    }
}

/// The figures of page number `page` (1-based), in points from the top left.
pub fn for_page(page: usize) -> Vec<legatorocr_pdf::Figure> {
    if let Ok(mut a) = ASKED.lock() {
        a.push(page);
    }
    let k = 72.0 / 25.4;
    CFG.lock()
        .map(|g| {
            g.iter()
                .filter(|s| s.page == page)
                .map(|s| legatorocr_pdf::Figure {
                    x: s.rect_mm[0] * k,
                    y: s.rect_mm[1] * k,
                    w: s.rect_mm[2] * k,
                    h: s.rect_mm[3] * k,
                    alt: s.alt.clone(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Figures whose page never came up while writing (a typo in the page number).
pub fn unmatched() -> Vec<Spec> {
    let asked = ASKED.lock().map(|a| a.clone()).unwrap_or_default();
    CFG.lock()
        .map(|g| {
            g.iter()
                .filter(|s| !asked.contains(&s.page))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_parse_and_refuse_missing_descriptions() {
        let s = parse("2,10,20,50,40=Chart of yearly rainfall").unwrap();
        assert_eq!((s.page, s.rect_mm), (2, [10.0, 20.0, 50.0, 40.0]));
        assert_eq!(s.alt, "Chart of yearly rainfall");
        assert!(parse("2,10,20,50,40").is_err());
        assert!(parse("2,10,20,50,40=  ").is_err());
        assert!(parse("0,10,20,50,40=x").is_err());
        assert!(parse("1,10,20,0,40=x").is_err());
        assert!(parse("1,10,20,50=x").is_err());
        set(vec![
            parse("3,1,1,5,5=a").unwrap(),
            parse("9,1,1,5,5=b").unwrap(),
        ]);
        assert_eq!(for_page(3).len(), 1);
        assert_eq!(
            unmatched().iter().map(|s| s.page).collect::<Vec<_>>(),
            vec![9]
        );
        set(vec![]);
    }
}
