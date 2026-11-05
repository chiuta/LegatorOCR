//! Sealed mode (`--offline`, or the environment variable `LEGATOROCR_OFFLINE=1`): the two
//! features of this program that reach the network (the time-stamp request with `curl`, and the
//! package-manager installs of `--install-modules`) refuse to run. Everything else here does not
//! use the network. This does not sandbox other programs the user points the engine at
//! (Tesseract, an ONNX bridge script, ...): it is a refusal in this program, not a firewall.
use std::sync::atomic::{AtomicBool, Ordering};

static ON: AtomicBool = AtomicBool::new(false);

pub fn set(on: bool) {
    ON.store(on, Ordering::SeqCst);
}

pub fn is_on() -> bool {
    ON.load(Ordering::SeqCst)
        || std::env::var("LEGATOROCR_OFFLINE").is_ok_and(|v| !v.is_empty() && v != "0")
}

/// `Err` with an explanation when sealed mode is on.
pub fn refuse(what: &str) -> anyhow::Result<()> {
    if is_on() {
        anyhow::bail!(
            "{what} needs the network and sealed mode (--offline / LEGATOROCR_OFFLINE) is on"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_mode_refuses_a_time_stamp_request() {
        set(true);
        let e = crate::tsa::request("https://example.invalid/tsa", &[0u8; 32]).unwrap_err();
        assert!(e.to_string().contains("sealed mode"), "{e}");
        set(false);
        assert!(refuse("x").is_ok());
    }
}
