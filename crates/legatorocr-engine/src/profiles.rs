//! Named setting bundles ("profiles"): a few built in, plus any `*.lgr` file in the
//! user's `legatorocr/profiles` folder. A profile is an ordinary project file without
//! sheets, applied on top of the default settings.

use crate::project::{Project, HEADER};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

/// Built-in profiles: (name, one-line description, body of the project file).
const BUILTIN: &[(&str, &str, &str)] = &[
    (
        "book-bw",
        "Text books: auto threshold, black & white, punch holes removed",
        "mode = bw\nthreshold = auto\npunch_holes = true\nauto_levels = true\n",
    ),
    (
        "book-small-color",
        "Colour books: sharp text over a light background, smallest files",
        "mode = hybrid\nhybrid_dpi = 100\nnormalize = true\nthreshold = auto\nsharpen = 0.4\npdf_quality = 70\n",
    ),
    (
        "document-text",
        "Office documents: sharp text, pictures kept in colour",
        "mode = mixed\nthreshold = auto\nauto_levels = true\nremove_lines = false\n",
    ),
    (
        "photo-archive",
        "Archive quality colour, light sharpening",
        "mode = color\nsharpen = 0.3\npdf_quality = 92\n",
    ),
    (
        "newspaper",
        "Newsprint: Wolf threshold, stronger speck removal, flattened curl",
        "mode = bw\nthreshold = wolf\ndewarp = true\ndespeckle_strength = 1.6\nauto_levels = true\n",
    ),
    (
        "form-dropout-red",
        "Forms with red rules or stamps: drop red, erase ruled lines",
        "mode = bw\nthreshold = auto\ndropout = red\nremove_lines = true\n",
    ),
    (
        "negatives",
        "Negative scans: invert and stretch contrast",
        "mode = gray\nauto_invert = true\nauto_levels = true\n",
    ),
    (
        "phone-photos",
        "Pages photographed by phone: level lighting, wider skew range, fingers removed",
        "mode = color\nnormalize = true\nauto_levels = true\nmax_skew = 15\nfingers = true\nsharpen = 0.5\n",
    ),
];

/// Names a profile may have (also the file stem of a user profile).
pub fn valid_name(n: &str) -> bool {
    !n.is_empty()
        && n.len() <= 48
        && n.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Folder with user profiles (not created here).
pub fn user_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    }?;
    Some(base.join("legatorocr").join("profiles"))
}

/// All available profiles as `(name, description)`: built-in first, then user ones
/// (a user file with a built-in's name overrides it).
pub fn list() -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = BUILTIN
        .iter()
        .map(|(n, d, _)| (n.to_string(), d.to_string()))
        .collect();
    if let Some(dir) = user_dir() {
        if let Ok(rd) = std::fs::read_dir(dir) {
            let mut names: Vec<String> = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    (p.extension().and_then(|x| x.to_str()) == Some("lgr"))
                        .then(|| p.file_stem()?.to_str().map(str::to_string))?
                })
                .filter(|n| valid_name(n))
                .collect();
            names.sort();
            for n in names {
                if !v.iter().any(|(x, _)| *x == n) {
                    v.push((n, "user profile".into()));
                }
            }
        }
    }
    v
}

/// Load a profile by name (user file first, then built-in).
pub fn load(name: &str) -> Result<Project> {
    if !valid_name(name) {
        bail!("invalid profile name '{name}' (letters, digits, - and _ only)");
    }
    if let Some(dir) = user_dir() {
        let f = dir.join(format!("{name}.lgr"));
        if f.is_file() {
            return Project::load(&f).with_context(|| format!("profile {name}"));
        }
    }
    match BUILTIN.iter().find(|(n, _, _)| *n == name) {
        Some((_, _, body)) => Project::parse(&format!("{HEADER}\n{body}"), None),
        None => bail!("unknown profile '{name}' (try --list-profiles)"),
    }
}

/// Save `p` (its settings and extras, no sheets) as a user profile; returns the file.
pub fn save_user(name: &str, p: &Project) -> Result<PathBuf> {
    if !valid_name(name) {
        bail!("invalid profile name '{name}' (letters, digits, - and _ only)");
    }
    let dir = user_dir().context("no configuration folder available on this system")?;
    std::fs::create_dir_all(&dir).with_context(|| dir.display().to_string())?;
    let f = dir.join(format!("{name}.lgr"));
    let bare = Project {
        settings: p.settings.clone(),
        extra: p.extra.clone(),
        sheets: Vec::new(),
    };
    bare.save(&f)?;
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;
    use legatorocr_core::OutputMode;

    #[test]
    fn every_builtin_parses_and_differs_from_default() {
        for (n, _, _) in BUILTIN {
            let p = load(n).unwrap_or_else(|e| panic!("{n}: {e:#}"));
            assert_ne!(
                crate::project::settings_to_pairs(&p.settings),
                crate::project::settings_to_pairs(&Default::default()),
                "{n} changes something"
            );
            assert!(p.sheets.is_empty());
        }
        assert_eq!(
            load("form-dropout-red").unwrap().settings.mode,
            OutputMode::BlackWhite
        );
        assert!(list().len() >= BUILTIN.len());
    }

    #[test]
    fn names_are_checked() {
        for bad in ["", "../x", "a b", "a/b", "é", &"x".repeat(60)] {
            assert!(!valid_name(bad), "{bad}");
            assert!(load(bad).is_err());
        }
        assert!(load("no-such-profile").is_err());
    }

    #[test]
    fn user_profile_round_trip_and_override() {
        let dir = std::env::temp_dir().join(format!("lgr-prof-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // XDG on Linux; elsewhere the test only checks the pure parts above.
        if cfg!(all(unix, not(target_os = "macos"))) {
            std::env::set_var("XDG_CONFIG_HOME", &dir);
            let mut p = load("book-bw").unwrap();
            p.settings.sharpen = 1.5;
            save_user("mine", &p).unwrap();
            save_user("newspaper", &p).unwrap(); // overrides the built-in
            assert_eq!(load("mine").unwrap().settings.sharpen, 1.5);
            assert_eq!(load("newspaper").unwrap().settings.sharpen, 1.5);
            assert!(list()
                .iter()
                .any(|(n, d)| n == "mine" && d == "user profile"));
            assert_eq!(list().iter().filter(|(n, _)| n == "newspaper").count(), 1);
            std::env::remove_var("XDG_CONFIG_HOME");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
