//! OCFL 1.1 packaging (<https://ocfl.io/1.1/spec/>): the files a run wrote become version `v1` of
//! one OCFL object, with a SHA-512 inventory, so a repository can later add versions without
//! losing this one. `verify` re-checks an object **that this program wrote** (it reads the
//! inventory by its own line layout, not as general JSON) and is not a full OCFL validator.
//! Not checked here against the reference validator (`ocfl-py` / the Go `ocfl` tool were not
//! available); the layout follows the specification text.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

pub struct Summary {
    pub files: usize,
    pub bytes: u64,
}

fn sha512_hex(data: &[u8]) -> String {
    legatorocr_pdf::crypt::sha512(data).iter().map(|b| format!("{b:02x}")).collect()
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// `2026-11-04T07:00:00Z` from Unix seconds.
fn rfc3339(when: u64) -> String {
    let z = (when / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let s = when % 86_400;
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", s / 3600, s % 3600 / 60, s % 60)
}

/// Write object `id` at `dest` (a new or empty folder) from `files` (flat, unique names).
pub fn write(dest: &Path, files: &[PathBuf], id: &str, when: u64) -> Result<Summary> {
    if id.trim().is_empty() {
        bail!("an OCFL object needs a non-empty id");
    }
    if dest.exists() && std::fs::read_dir(dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
        bail!("{} exists and is not empty; an OCFL object is written into a new folder", dest.display());
    }
    let content = dest.join("v1").join("content");
    std::fs::create_dir_all(&content).with_context(|| format!("{}: cannot create", content.display()))?;
    // digest -> (content path, logical names); a repeated content is stored once
    let mut by_digest: Vec<(String, String, Vec<String>)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut bytes = 0u64;
    for f in files {
        let name = f
            .file_name()
            .and_then(|n| n.to_str())
            .with_context(|| format!("{}: unusable file name", f.display()))?;
        if !seen.insert(name.to_string()) {
            bail!("two files are named {name}; an object written here keeps one flat folder");
        }
        let data = std::fs::read(f).with_context(|| format!("{}: cannot read", f.display()))?;
        bytes += data.len() as u64;
        let d = sha512_hex(&data);
        match by_digest.iter_mut().find(|e| e.0 == d) {
            Some(e) => e.2.push(name.to_string()),
            None => {
                std::fs::write(content.join(name), &data)?;
                by_digest.push((d, format!("v1/content/{name}"), vec![name.to_string()]));
            }
        }
    }
    let mut inv = String::from("{\n");
    inv.push_str(&format!("  \"id\": {},\n", json_str(id)));
    inv.push_str("  \"type\": \"https://ocfl.io/1.1/spec/#inventory\",\n  \"digestAlgorithm\": \"sha512\",\n  \"head\": \"v1\",\n");
    inv.push_str("  \"manifest\": {\n");
    for (k, (d, p, _)) in by_digest.iter().enumerate() {
        inv.push_str(&format!("    \"{d}\": [{}]{}\n", json_str(p), if k + 1 < by_digest.len() { "," } else { "" }));
    }
    inv.push_str("  },\n  \"versions\": {\n    \"v1\": {\n");
    inv.push_str(&format!("      \"created\": \"{}\",\n", rfc3339(when)));
    inv.push_str(&format!(
        "      \"message\": {},\n",
        json_str(&format!("Written by LegatorOCR {}", env!("CARGO_PKG_VERSION")))
    ));
    inv.push_str("      \"state\": {\n");
    for (k, (d, _, names)) in by_digest.iter().enumerate() {
        let list: Vec<String> = names.iter().map(|n| json_str(n)).collect();
        inv.push_str(&format!("        \"{d}\": [{}]{}\n", list.join(", "), if k + 1 < by_digest.len() { "," } else { "" }));
    }
    inv.push_str("      }\n    }\n  }\n}\n");
    let side = format!("{}  inventory.json\n", sha512_hex(inv.as_bytes()));
    for dir in [dest.to_path_buf(), dest.join("v1")] {
        std::fs::write(dir.join("inventory.json"), &inv)?;
        std::fs::write(dir.join("inventory.json.sha512"), &side)?;
    }
    std::fs::write(dest.join("0=ocfl_object_1.1"), "ocfl_object_1.1\n")?;
    Ok(Summary { files: files.len(), bytes })
}

/// Problems found in an object written by [`write`] (empty = intact).
pub fn verify(dir: &Path) -> Result<Vec<String>> {
    let mut problems = Vec::new();
    match std::fs::read_to_string(dir.join("0=ocfl_object_1.1")) {
        Ok(t) if t == "ocfl_object_1.1\n" => {}
        _ => problems.push("0=ocfl_object_1.1 is missing or wrong".into()),
    }
    let inv = std::fs::read(dir.join("inventory.json")).with_context(|| format!("{}: no inventory.json", dir.display()))?;
    let side = std::fs::read_to_string(dir.join("inventory.json.sha512")).unwrap_or_default();
    if side.split_whitespace().next() != Some(sha512_hex(&inv).as_str()) {
        problems.push("inventory.json does not match inventory.json.sha512".into());
    }
    match std::fs::read(dir.join("v1").join("inventory.json")) {
        Ok(v1) if v1 == inv => {}
        _ => problems.push("v1/inventory.json differs from the root inventory".into()),
    }
    let text = String::from_utf8_lossy(&inv).to_string();
    let mut in_manifest = false;
    let mut n = 0;
    for l in text.lines() {
        let t = l.trim();
        if t.starts_with("\"manifest\"") {
            in_manifest = true;
            continue;
        }
        if in_manifest && t.starts_with('}') {
            break;
        }
        if in_manifest {
            let Some((d, rest)) = t.trim_start_matches('"').split_once("\": [\"") else {
                problems.push(format!("unreadable manifest line: {t}"));
                continue;
            };
            let path = rest.split("\"]").next().unwrap_or("");
            n += 1;
            if path.contains("..") || Path::new(path).is_absolute() {
                problems.push(format!("{path}: path leaves the object"));
                continue;
            }
            match std::fs::read(dir.join(path)) {
                Ok(b) if sha512_hex(&b) == d => {}
                Ok(_) => problems.push(format!("{path}: content changed")),
                Err(_) => problems.push(format!("{path}: missing")),
            }
        }
    }
    if n == 0 {
        problems.push("the manifest lists no files".into());
    }
    let listed: std::collections::HashSet<String> = text
        .lines()
        .filter_map(|l| l.split("\"v1/content/").nth(1).and_then(|r| r.split('"').next()).map(str::to_string))
        .collect();
    if let Ok(rd) = std::fs::read_dir(dir.join("v1").join("content")) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if !listed.contains(&name) {
                problems.push(format!("v1/content/{name}: not in the manifest"));
            }
        }
    }
    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn written_object_verifies_and_changes_are_found() {
        let d = std::env::temp_dir().join(format!("lgr-ocfl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let (a, b, c) = (d.join("a.txt"), d.join("b.txt"), d.join("c.pdf"));
        std::fs::write(&a, "same").unwrap();
        std::fs::write(&b, "same").unwrap(); // duplicate content: stored once
        std::fs::write(&c, "other \"quoted\"").unwrap();
        let obj = d.join("obj");
        let s = write(&obj, &[a, b, c], "urn:test:1", 1_793_000_000).unwrap();
        assert_eq!(s.files, 3);
        assert_eq!(std::fs::read_dir(obj.join("v1/content")).unwrap().count(), 2);
        assert!(verify(&obj).unwrap().is_empty(), "{:?}", verify(&obj));
        let inv = std::fs::read_to_string(obj.join("inventory.json")).unwrap();
        assert!(inv.contains("\"digestAlgorithm\": \"sha512\"") && inv.contains("\"head\": \"v1\""));
        assert!(inv.contains("\"created\": \"2026-10-26T07:33:20Z\""), "{inv}");
        std::fs::write(obj.join("v1/content/c.pdf"), "tampered").unwrap();
        assert!(verify(&obj).unwrap().iter().any(|p| p.contains("c.pdf: content changed")));
        std::fs::write(obj.join("v1/content/extra.txt"), "x").unwrap();
        assert!(verify(&obj).unwrap().iter().any(|p| p.contains("extra.txt")));
        assert!(write(&obj, &[], "x", 0).is_err(), "a non-empty folder is refused");
        let _ = std::fs::remove_dir_all(&d);
    }
}
