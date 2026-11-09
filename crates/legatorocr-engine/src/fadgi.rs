//! Star levels (1 to 4) for measurements taken on a scan of unbound documents, general
//! collections, following one table of the FADGI Technical Guidelines (2016 edition). The
//! measurements are taken as given: this module does not measure anything.
//!
//! Not covered: the OECF and white-balance rows (the notation of their limits is not
//! understood), any category other than unbound documents, a single overall level (the rule
//! that combines the rows is not known) and the newer edition of the guidelines.

use anyhow::{bail, Result};

/// The only supported material category.
pub const CATEGORY: &str = "documents-unbound-general";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Measurements {
    /// Resolution in pixels per inch.
    pub ppi: Option<f64>,
    /// Illumination non-uniformity, percent.
    pub uniformity_pct: Option<f64>,
    /// Mean colour error, CIEDE2000.
    pub delta_e_mean: Option<f64>,
    /// Sampling efficiency from MTF10, percent.
    pub sampling_efficiency_pct: Option<f64>,
    /// SFR at half the sampling frequency (a ratio, 0 to 1).
    pub sfr_at_half_sampling: Option<f64>,
    /// MTF50 as a percentage of half the sampling frequency.
    pub mtf50_pct_of_half_sampling: Option<f64>,
    /// Error of the reproduction scale, percent of the target (signed).
    pub scale_error_pct: Option<f64>,
    /// Largest MTF value (sharpening).
    pub sharpening_max_mtf: Option<f64>,
    /// Noise, standard deviation of L* in 8-bit levels.
    pub noise_levels: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// No value was given.
    NotMeasured,
    /// Stars reached: 0 means below one star.
    Stars(u8),
    /// The row exists in the table but is not evaluated here.
    NotEvaluated(&'static str),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub name: &'static str,
    pub value: Option<f64>,
    pub outcome: Outcome,
    pub note: Option<&'static str>,
}

#[derive(Debug)]
pub struct Report {
    pub rows: Vec<Row>,
    /// Rows holding the lowest star count among the evaluated rows.
    pub limiting: Vec<&'static str>,
    pub notes: Vec<&'static str>,
}

/// Highest level (1 to 4) for which `ok(level_index)` holds; 0 when none does.
fn highest(ok: impl Fn(usize) -> bool) -> u8 {
    (0..4)
        .rev()
        .find(|&i| ok(i))
        .map(|i| i as u8 + 1)
        .unwrap_or(0)
}

fn row(
    name: &'static str,
    value: Option<f64>,
    note: Option<&'static str>,
    ok: impl Fn(f64, usize) -> bool,
) -> Row {
    let outcome = match value {
        None => Outcome::NotMeasured,
        Some(v) if v.is_finite() => Outcome::Stars(highest(|i| ok(v, i))),
        Some(_) => Outcome::NotMeasured,
    };
    Row {
        name,
        value,
        outcome,
        note,
    }
}

/// Place the measurements in the levels of the table for `category`.
pub fn classify(category: &str, m: &Measurements) -> Result<Report> {
    if category != CATEGORY {
        bail!("category {category:?} is not supported; only {CATEGORY} (unbound documents, general collections)");
    }
    let mut rows = vec![
        row(
            "resolution (ppi)",
            m.ppi,
            Some("300 ppi is the value of both 2 and 3 stars; the higher is reported"),
            |v, i| v >= [150.0, 300.0, 300.0, 400.0][i],
        ),
        Row {
            name: "tonal response (OECF)",
            value: None,
            outcome: Outcome::NotEvaluated("the notation of the limits is not understood"),
            note: None,
        },
        Row {
            name: "white balance error",
            value: None,
            outcome: Outcome::NotEvaluated("the notation of the limits is not understood"),
            note: None,
        },
        row(
            "illumination non-uniformity (%)",
            m.uniformity_pct,
            None,
            |v, i| v < [8.0, 5.0, 3.0, 1.0][i],
        ),
        row(
            "colour accuracy, mean deltaE2000",
            m.delta_e_mean,
            Some("only a mean limit, no maximum"),
            |v, i| v < [10.0, 8.0, 5.0, 4.0][i],
        ),
        row(
            "MTF10 sampling efficiency (%)",
            m.sampling_efficiency_pct,
            None,
            |v, i| v > [60.0, 70.0, 80.0, 90.0][i],
        ),
        row(
            "SFR at half the sampling frequency",
            m.sfr_at_half_sampling,
            Some("the same limit at every level"),
            |v, _| v < 0.4,
        ),
        row(
            "MTF50 (% of half the sampling frequency)",
            m.mtf50_pct_of_half_sampling,
            Some("closed range"),
            |v, i| {
                let (a, b) = [(30.0, 85.0), (30.0, 85.0), (35.0, 75.0), (40.0, 65.0)][i];
                v >= a && v <= b
            },
        ),
        row(
            "reproduction scale accuracy (% of target)",
            m.scale_error_pct,
            Some("limit taken as inclusive"),
            |v, i| v.abs() <= [5.0, 3.0, 2.0, 1.0][i],
        ),
        row(
            "sharpening (maximum MTF)",
            m.sharpening_max_mtf,
            None,
            |v, i| {
                if i == 3 {
                    v <= 1.0
                } else {
                    v < [1.3, 1.2, 1.1][i]
                }
            },
        ),
        row(
            "noise (std deviation of L*, levels)",
            m.noise_levels,
            Some("first limit of each pair, strict; the second figure is not understood"),
            |v, i| v < [6.0, 5.0, 4.0, 3.0][i],
        ),
    ];
    let min = rows
        .iter()
        .filter_map(|r| match r.outcome {
            Outcome::Stars(s) => Some(s),
            _ => None,
        })
        .min();
    let limiting = match min {
        Some(mn) => rows
            .iter()
            .filter(|r| r.outcome == Outcome::Stars(mn))
            .map(|r| r.name)
            .collect(),
        None => Vec::new(),
    };
    rows.shrink_to_fit();
    Ok(Report {
        rows,
        limiting,
        notes: vec![
            "table: FADGI Technical Guidelines 2016, documents (unbound), general collections; the 2023 edition may differ",
            "the noise row of the source table shows '>' where the other tables show '<'; '<' is used",
            "no overall level is given: the rule that combines the rows is not known",
        ],
    })
}

/// Parse `name=value,...` (names: ppi, uniformity, deltae, efficiency, sfr-half, mtf50-pct,
/// scale-error, sharpening, noise). `dpi` is refused: the table is in pixels per inch.
pub fn parse_measurements(list: &str) -> Result<Measurements> {
    let mut m = Measurements::default();
    for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let Some((k, v)) = part.split_once('=') else {
            bail!("expected name=value, got {part:?}");
        };
        let v: f64 = v
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("{part:?}: the value is not a number"))?;
        let slot = match k.trim() {
            "ppi" => &mut m.ppi,
            "dpi" => bail!(
                "dpi is not accepted: give the resolution as ppi (pixels per inch of the image)"
            ),
            "uniformity" => &mut m.uniformity_pct,
            "deltae" => &mut m.delta_e_mean,
            "efficiency" => &mut m.sampling_efficiency_pct,
            "sfr-half" => &mut m.sfr_at_half_sampling,
            "mtf50-pct" => &mut m.mtf50_pct_of_half_sampling,
            "scale-error" => &mut m.scale_error_pct,
            "sharpening" => &mut m.sharpening_max_mtf,
            "noise" => &mut m.noise_levels,
            other => bail!("unknown measurement {other:?}"),
        };
        *slot = Some(v);
    }
    Ok(m)
}

pub fn render(r: &Report) -> String {
    let mut s = String::new();
    for row in &r.rows {
        let v = row
            .value
            .map(|x| format!("{x}"))
            .unwrap_or_else(|| "-".into());
        let o = match &row.outcome {
            Outcome::NotMeasured => "not measured".to_string(),
            Outcome::Stars(0) => "below one star".to_string(),
            Outcome::Stars(n) => format!("{n} star(s)"),
            Outcome::NotEvaluated(why) => format!("not evaluated ({why})"),
        };
        s.push_str(&format!("{:<44} {:>8}  {o}\n", row.name, v));
    }
    if !r.limiting.is_empty() {
        s.push_str(&format!("limiting: {}\n", r.limiting.join("; ")));
    }
    for n in &r.notes {
        s.push_str(&format!("note: {n}\n"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stars(m: &Measurements, name_start: &str) -> Outcome {
        classify(CATEGORY, m)
            .unwrap()
            .rows
            .into_iter()
            .find(|r| r.name.starts_with(name_start))
            .unwrap()
            .outcome
    }

    #[test]
    fn resolution() {
        let f = |v| {
            stars(
                &Measurements {
                    ppi: Some(v),
                    ..Measurements::default()
                },
                "resolution",
            )
        };
        assert_eq!(f(150.0), Outcome::Stars(1));
        assert_eq!(f(300.0), Outcome::Stars(3));
        assert_eq!(f(400.0), Outcome::Stars(4));
        assert_eq!(f(100.0), Outcome::Stars(0));
    }

    #[test]
    fn uniformity_delta_e_and_efficiency() {
        let u = |v| {
            stars(
                &Measurements {
                    uniformity_pct: Some(v),
                    ..Default::default()
                },
                "illumination",
            )
        };
        assert_eq!(
            (u(2.0), u(0.5), u(9.0), u(3.0)),
            (
                Outcome::Stars(3),
                Outcome::Stars(4),
                Outcome::Stars(0),
                Outcome::Stars(2)
            )
        );
        let d = |v| {
            stars(
                &Measurements {
                    delta_e_mean: Some(v),
                    ..Default::default()
                },
                "colour",
            )
        };
        assert_eq!(
            (d(6.0), d(4.5), d(3.9)),
            (Outcome::Stars(2), Outcome::Stars(3), Outcome::Stars(4))
        );
        let e = |v| {
            stars(
                &Measurements {
                    sampling_efficiency_pct: Some(v),
                    ..Default::default()
                },
                "MTF10",
            )
        };
        assert_eq!(
            (e(85.0), e(92.0), e(55.0), e(90.0)),
            (
                Outcome::Stars(3),
                Outcome::Stars(4),
                Outcome::Stars(0),
                Outcome::Stars(3)
            )
        );
    }

    #[test]
    fn mtf50_ranges_are_closed() {
        let f = |v| {
            stars(
                &Measurements {
                    mtf50_pct_of_half_sampling: Some(v),
                    ..Default::default()
                },
                "MTF50",
            )
        };
        assert_eq!(
            (f(50.0), f(70.0), f(80.0), f(20.0)),
            (
                Outcome::Stars(4),
                Outcome::Stars(3),
                Outcome::Stars(2),
                Outcome::Stars(0)
            )
        );
        assert_eq!(
            (f(40.0), f(65.0), f(30.0), f(85.0), f(85.1)),
            (
                Outcome::Stars(4),
                Outcome::Stars(4),
                Outcome::Stars(2),
                Outcome::Stars(2),
                Outcome::Stars(0)
            )
        );
    }

    #[test]
    fn scale_sharpening_noise_and_sfr() {
        let s = |v| {
            stars(
                &Measurements {
                    scale_error_pct: Some(v),
                    ..Default::default()
                },
                "reproduction",
            )
        };
        assert_eq!(
            (s(-0.8), s(2.5), s(4.0), s(6.0)),
            (
                Outcome::Stars(4),
                Outcome::Stars(2),
                Outcome::Stars(1),
                Outcome::Stars(0)
            )
        );
        let h = |v| {
            stars(
                &Measurements {
                    sharpening_max_mtf: Some(v),
                    ..Default::default()
                },
                "sharpening",
            )
        };
        assert_eq!(
            (h(1.0), h(1.05), h(1.15), h(1.25), h(1.3)),
            (
                Outcome::Stars(4),
                Outcome::Stars(3),
                Outcome::Stars(2),
                Outcome::Stars(1),
                Outcome::Stars(0)
            )
        );
        let n = |v| {
            stars(
                &Measurements {
                    noise_levels: Some(v),
                    ..Default::default()
                },
                "noise",
            )
        };
        assert_eq!(
            (n(2.9), n(3.0), n(5.5), n(6.0)),
            (
                Outcome::Stars(4),
                Outcome::Stars(3),
                Outcome::Stars(1),
                Outcome::Stars(0)
            )
        );
        let f = |v| {
            stars(
                &Measurements {
                    sfr_at_half_sampling: Some(v),
                    ..Default::default()
                },
                "SFR",
            )
        };
        assert_eq!((f(0.39), f(0.4)), (Outcome::Stars(4), Outcome::Stars(0)));
    }

    #[test]
    fn missing_values_are_not_measured_and_do_not_degrade_the_others() {
        let m = Measurements {
            ppi: Some(400.0),
            uniformity_pct: Some(0.5),
            ..Default::default()
        };
        let r = classify(CATEGORY, &m).unwrap();
        assert_eq!(
            r.rows
                .iter()
                .find(|x| x.name.starts_with("colour"))
                .unwrap()
                .outcome,
            Outcome::NotMeasured
        );
        assert_eq!(
            r.limiting.len(),
            2,
            "both measured rows hold 4 stars: {:?}",
            r.limiting
        );
        let m2 = Measurements {
            delta_e_mean: Some(9.0),
            ..m
        };
        let r2 = classify(CATEGORY, &m2).unwrap();
        assert_eq!(r2.limiting, vec!["colour accuracy, mean deltaE2000"]);
        assert!(render(&r2).contains("not measured") && render(&r2).contains("limiting:"));
    }

    #[test]
    fn category_and_units_are_checked() {
        assert!(classify("photographs", &Measurements::default()).is_err());
        assert!(parse_measurements("dpi=300").is_err());
        assert!(parse_measurements("ppi=abc").is_err());
        assert!(parse_measurements("nope=1").is_err());
        let m = parse_measurements("ppi=300, deltae=6").unwrap();
        assert_eq!((m.ppi, m.delta_e_mean), (Some(300.0), Some(6.0)));
        // the typo note is carried in every report
        assert!(classify(CATEGORY, &m)
            .unwrap()
            .notes
            .iter()
            .any(|n| n.contains("'>'")));
    }
}
