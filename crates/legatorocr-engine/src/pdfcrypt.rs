//! `--object-streams` (smaller files) and `--encrypt-pdf`: both rewrite the finished PDF. Encryption: password-protect the PDFs this program writes (AES-256). The user password
//! opens the file; the owner password lifts the restrictions listed by `--pdf-permissions`.
//! Passwords come from the environment (LEGATOROCR_PDF_USER_PASSWORD, LEGATOROCR_PDF_OWNER_PASSWORD),
//! never from the command line.

use anyhow::{bail, Result};
use legatorocr_pdf::reader::encrypt::{perm, rewrite, Protect};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

static COMPACT: AtomicBool = AtomicBool::new(false);
static LINEAR: AtomicBool = AtomicBool::new(false);

/// `--linearize`: Fast Web View order (first page first, hint tables).
pub fn set_linearize(on: bool) {
    LINEAR.store(on, Ordering::Relaxed);
}

/// `--object-streams`: pack plain objects into compressed object streams (PDF 1.5).
pub fn set_compact(on: bool) {
    COMPACT.store(on, Ordering::Relaxed);
}

struct Cfg {
    user: String,
    owner: String,
    allowed: i32,
}

static CFG: Mutex<Option<Cfg>> = Mutex::new(None);

/// `print,copy,...` → permission bits. `all` and `none` are accepted; the empty string is the
/// default (print, high-quality print, accessibility).
pub fn parse_perms(spec: &str) -> Result<i32> {
    if spec.trim().is_empty() {
        return Ok(perm::PRINT | perm::HIGH_RES | perm::ACCESSIBILITY);
    }
    let mut bits = 0;
    for w in spec.split(',').map(|w| w.trim().to_ascii_lowercase()) {
        bits |= match w.as_str() {
            "all" => perm::ALL,
            "none" => 0,
            "print" => perm::PRINT,
            "highres" => perm::HIGH_RES | perm::PRINT,
            "modify" => perm::MODIFY,
            "copy" => perm::COPY,
            "annotate" => perm::ANNOTATE,
            "forms" => perm::FORMS,
            "accessibility" => perm::ACCESSIBILITY,
            "assemble" => perm::ASSEMBLE,
            "" => 0,
            other => bail!(
                "unknown permission {other:?}; use print, highres, modify, copy, annotate, forms, accessibility, assemble, all or none"
            ),
        };
    }
    Ok(bits)
}

/// Turn encryption on for everything written from now on (`None` turns it off).
pub fn set(user: &str, owner: &str, allowed: i32) -> Result<()> {
    if user.is_empty() && owner.is_empty() {
        bail!(
            "--encrypt-pdf needs a password: set LEGATOROCR_PDF_USER_PASSWORD (to open the file) \
             and/or LEGATOROCR_PDF_OWNER_PASSWORD (to lift the restrictions)"
        );
    }
    if user.is_empty() && allowed == perm::ALL {
        bail!("an owner password without any restriction protects nothing; narrow --pdf-permissions or set a user password");
    }
    *CFG.lock().unwrap() = Some(Cfg {
        user: user.to_string(),
        owner: owner.to_string(),
        allowed,
    });
    Ok(())
}

pub fn clear() {
    *CFG.lock().unwrap() = None;
}

pub fn active() -> bool {
    CFG.lock().unwrap().is_some()
        || COMPACT.load(Ordering::Relaxed)
        || LINEAR.load(Ordering::Relaxed)
}

/// Encrypt `bytes` when [`set`] was called, else hand them back unchanged.
pub fn apply(bytes: Vec<u8>) -> Result<Vec<u8>> {
    let g = CFG.lock().unwrap();
    let compact = COMPACT.load(Ordering::Relaxed);
    if LINEAR.load(Ordering::Relaxed) {
        if g.is_some() || compact {
            bail!("--linearize cannot be combined with --encrypt-pdf or --object-streams");
        }
        return legatorocr_pdf::reader::linearize::linearize(&bytes)
            .map_err(|e| anyhow::anyhow!("{e}"));
    }
    if g.is_none() && !compact {
        return Ok(bytes);
    }
    let p = g.as_ref().map(|c| Protect {
        user: &c.user,
        owner: &c.owner,
        allowed: c.allowed,
    });
    let mut rng = |b: &mut [u8]| {
        getrandom::fill(b).expect("no random numbers from the system");
    };
    rewrite(&bytes, compact, p.as_ref(), &mut rng).map_err(|e| anyhow::anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_parse() {
        assert_eq!(parse_perms("none").unwrap(), 0);
        assert_eq!(parse_perms("all").unwrap(), perm::ALL);
        assert_eq!(
            parse_perms("copy, print").unwrap(),
            perm::COPY | perm::PRINT
        );
        assert!(parse_perms("fly").is_err());
        assert_ne!(parse_perms("").unwrap() & perm::PRINT, 0);
    }

    #[test]
    fn needs_a_password() {
        assert!(set("", "", 0).is_err());
        assert!(set("", "o", perm::ALL).is_err());
    }
}
