//! Rights notice from the author's year of death, for Romanian law: copyright lasts for the
//! author's life and 70 years after death, counted from 1 January of the year after the death
//! (Law 8/1996, art. 25, as amended). This covers the plain case only. It is information, not
//! legal advice: anonymous or collective works, translations, posthumous publication, works of
//! several authors and neighbouring rights follow other rules and must be checked separately.

use anyhow::{bail, Result};

/// The year in which the work enters the public domain (1 January of that year).
pub fn public_domain_year(death_year: i32) -> i32 {
    death_year + 71
}

/// Notice text for `death_year`, judged at `today_year`.
/// `deed`: assignment deed number and date, printed as written.
pub fn notice(death_year: i32, today_year: i32, author: Option<&str>, deed: Option<(&str, &str)>) -> Result<String> {
    if !(1000..=today_year).contains(&death_year) {
        bail!("--rights-death-year {death_year}: expected a year between 1000 and {today_year}");
    }
    let pd = public_domain_year(death_year);
    let who = author.map(|a| format!("{a}, ")).unwrap_or_default();
    let status = if today_year >= pd {
        format!("In the public domain in Romania since 1 January {pd}")
    } else {
        format!(
            "Protected by copyright in Romania until 31 December {}; enters the public domain on 1 January {pd}",
            pd - 1
        )
    };
    let deed = match deed {
        Some((n, d)) => format!(" Rights assigned by deed no. {n} of {d}."),
        None => String::new(),
    };
    Ok(format!(
        "{status} (author {who}died {death_year}; life + 70 years, Law 8/1996).{deed} Information only: check anonymous, collective, translated or posthumous works and other countries separately."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_runs_from_the_year_after_death() {
        assert_eq!(public_domain_year(1955), 2026);
        assert!(notice(1955, 2026, None, None).unwrap().contains("public domain in Romania since 1 January 2026"));
        let n = notice(1956, 2026, Some("I. Autor"), Some(("12/B", "3 March 2026"))).unwrap();
        assert!(n.contains("until 31 December 2026") && n.contains("I. Autor"), "{n}");
        assert!(n.contains("deed no. 12/B of 3 March 2026"));
        assert!(notice(2030, 2026, None, None).is_err());
        assert!(notice(12, 2026, None, None).is_err());
    }
}
