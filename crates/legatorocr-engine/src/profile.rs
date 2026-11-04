//! `--profile`: wall time per stage (summed over all pages and threads) and peak memory.
//!
//! Disabled by default; when disabled a stage costs one atomic load. Times are the sum over
//! worker threads, so with N threads the total can exceed the elapsed time.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

static ON: AtomicBool = AtomicBool::new(false);
static STAGES: Mutex<BTreeMap<&'static str, (Duration, u64)>> = Mutex::new(BTreeMap::new());

/// Turn collection on or off (and clear what was collected).
pub fn enable(on: bool) {
    ON.store(on, Ordering::Relaxed);
    if on {
        // finer stages inside the clean-up step ("clean-up/deskew", ...)
        let _ = legatorocr_core::STAGE_HOOK.set(record);
    }
    if let Ok(mut m) = STAGES.lock() {
        m.clear();
    }
}

pub fn enabled() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Run `f`, charging its wall time to `stage` when profiling is on.
pub fn time<T>(stage: &'static str, f: impl FnOnce() -> T) -> T {
    if !enabled() {
        return f();
    }
    let t = Instant::now();
    let r = f();
    record(stage, t.elapsed());
    r
}

pub fn record(stage: &'static str, d: Duration) {
    if let Ok(mut m) = STAGES.lock() {
        let e = m.entry(stage).or_insert((Duration::ZERO, 0));
        e.0 += d;
        e.1 += 1;
    }
}

/// Peak resident memory in bytes, where the platform offers it (Linux); `None` elsewhere.
pub fn peak_memory() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// Text report; empty when nothing was collected.
pub fn report() -> String {
    let m = match STAGES.lock() {
        Ok(m) => m.clone(),
        Err(_) => return String::new(),
    };
    if m.is_empty() {
        return String::new();
    }
    let total: Duration = m
        .iter()
        .filter(|(k, _)| !k.contains('/'))
        .map(|v| v.1 .0)
        .sum();
    let mut out =
        String::from("stage                  calls     time (s)   share   per call (ms)\n");
    let mut rows: Vec<_> = m.into_iter().collect();
    rows.sort_by_key(|b| std::cmp::Reverse(b.1 .0));
    for (name, (d, n)) in rows {
        let name = if name.contains('/') {
            format!("  {name}")
        } else {
            name.to_string()
        };
        let share = if total.is_zero() {
            0.0
        } else {
            d.as_secs_f64() / total.as_secs_f64() * 100.0
        };
        out.push_str(&format!(
            "{name:<22} {n:>7} {:>12.2} {share:>6.1}% {:>14.1}\n",
            d.as_secs_f64(),
            d.as_secs_f64() * 1000.0 / n.max(1) as f64
        ));
    }
    out.push_str(&format!(
        "total (summed over threads): {:.2} s\n",
        total.as_secs_f64()
    ));
    if let Some(b) = peak_memory() {
        out.push_str(&format!("peak memory: {:.0} MiB\n", b as f64 / 1048576.0));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the collector is global state.
    #[test]
    fn collects_only_when_enabled_and_reports_shares() {
        enable(false);
        assert_eq!(time("a", || 7), 7);
        assert!(report().is_empty());
        enable(true);
        time("slow", || std::thread::sleep(Duration::from_millis(30)));
        time("fast", || ());
        time("fast", || ());
        let r = report();
        assert!(r.contains("slow"), "{r}");
        assert!(r.contains("fast"), "{r}");
        assert!(
            r.lines()
                .find(|l| l.starts_with("fast"))
                .unwrap()
                .contains(" 2 "),
            "{r}"
        );
        enable(false);
        assert!(report().is_empty());
    }
}
