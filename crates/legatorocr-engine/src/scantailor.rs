//! Reads the page list of a ScanTailor project (`.ScanTailor`, XML) so that an existing project can be
//! opened here: the scan files and the per-image rotation are taken over; ScanTailor's own skew,
//! split and content boxes are NOT (the sign conventions are not verified), so those are found
//! again by this program.
//!
//! The reader is deliberately tolerant: it looks only at a few element names and attributes
//! (`directory`/`file`/`image`/`page` and a `rotation degrees=` inside the orientation filter) and
//! ignores everything else. The format was reconstructed from the published description of the
//! project file, not checked against project files written by every ScanTailor version.

use crate::project::{Project, SheetAdj};
use anyhow::{bail, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// True when the text looks like an XML project (ScanTailor writes `<?xml` then `<project`).
pub fn looks_like_scantailor(text: &str) -> bool {
    let t = text.trim_start();
    t.starts_with("<?xml") && t.contains("<project")
}

struct Tag {
    name: String,
    attrs: Vec<(String, String)>,
    closing: bool,
}

fn unescape(v: &str) -> String {
    v.replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn tags(text: &str) -> Vec<Tag> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(end) = text[i..].find('>') else { break };
        let body = &text[i + 1..i + end];
        i += end + 1;
        if body.starts_with('?') || body.starts_with('!') {
            continue;
        }
        let closing = body.starts_with('/');
        let body = body.trim_start_matches('/').trim_end_matches('/');
        let mut parts = body.splitn(2, char::is_whitespace);
        let name = parts.next().unwrap_or("").replace('-', "_");
        let mut attrs = Vec::new();
        let mut rest = parts.next().unwrap_or("");
        while let Some(eq) = rest.find('=') {
            let key = rest[..eq].trim().to_string();
            let after = rest[eq + 1..].trim_start();
            let Some(q) = after.chars().next().filter(|c| *c == '"' || *c == '\'') else { break };
            let Some(close) = after[1..].find(q) else { break };
            attrs.push((key, unescape(&after[1..1 + close])));
            rest = &after[close + 2..];
        }
        out.push(Tag { name, attrs, closing });
    }
    out
}

fn attr<'a>(t: &'a Tag, k: &str) -> Option<&'a str> {
    t.attrs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
}

pub fn parse(text: &str, base: Option<&Path>) -> Result<Project> {
    if !looks_like_scantailor(text) {
        bail!("not a ScanTailor project (no <project> element)");
    }
    let mut dirs: HashMap<String, String> = HashMap::new();
    let mut files: HashMap<String, (String, String)> = HashMap::new(); // id -> (dirId, name)
    let mut images: Vec<(String, String)> = Vec::new(); // (image id, file id) in file order
    let mut rotation: HashMap<String, u16> = HashMap::new(); // image id -> degrees
    let mut section = String::new();
    let mut cur_image: Option<String> = None;
    for t in tags(text) {
        if t.closing {
            if t.name == "image" && section == "fix_orientation" {
                cur_image = None;
            }
            if t.name == section {
                section.clear();
            }
            continue;
        }
        match t.name.as_str() {
            "directories" | "files" | "images" | "pages" | "filters" => section = t.name.clone(),
            "fix_orientation" => section = "fix_orientation".into(),
            "directory" => {
                if let (Some(id), Some(p)) = (attr(&t, "id"), attr(&t, "path")) {
                    dirs.insert(id.into(), p.into());
                }
            }
            "file" if section == "files" => {
                if let (Some(id), Some(d), Some(n)) = (attr(&t, "id"), attr(&t, "dirId"), attr(&t, "name")) {
                    files.insert(id.into(), (d.into(), n.into()));
                }
            }
            "image" if section == "images" => {
                if let (Some(id), Some(f)) = (attr(&t, "id"), attr(&t, "fileId")) {
                    images.push((id.into(), f.into()));
                }
            }
            "image" if section == "fix_orientation" || section == "filters" => {
                cur_image = attr(&t, "id").map(str::to_string);
            }
            "rotation" => {
                if let (Some(id), Some(d)) = (&cur_image, attr(&t, "degrees")) {
                    if let Ok(d) = d.parse::<f32>() {
                        rotation.insert(id.clone(), (d.round() as i32).rem_euclid(360) as u16);
                    }
                }
            }
            _ => {}
        }
    }
    if images.is_empty() {
        bail!("the ScanTailor project lists no images");
    }
    let mut p = Project::default();
    for (id, fid) in images {
        let Some((did, name)) = files.get(&fid) else { continue };
        let dir = dirs.get(did).map(PathBuf::from).unwrap_or_default();
        let mut path = if dir.as_os_str().is_empty() { PathBuf::from(name) } else { dir.join(name) };
        if path.is_relative() {
            if let Some(b) = base {
                path = b.join(path);
            }
        }
        let adj = SheetAdj { rotate_quarters: (rotation.get(&id).copied().unwrap_or(0) / 90 % 4) as u8, ..Default::default() };
        p.sheets.push((path, adj));
    }
    if p.sheets.is_empty() {
        bail!("none of the ScanTailor images could be matched to a file");
    }
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<project layoutDirection="LTR" outputDirectory="/out" version="3">
 <directories><directory id="1" path="/scans"/></directories>
 <files><file id="2" dirId="1" name="a.tif"/><file id="3" dirId="1" name="b &amp; c.tif"/></files>
 <images><image id="4" fileId="2" fileImageNo="0"/><image id="5" fileId="3" fileImageNo="0"/></images>
 <pages><page id="6" imageId="4" subPage="SINGLE_PAGE"/><page id="7" imageId="5" subPage="SINGLE_PAGE"/></pages>
 <filters><fix-orientation><image id="5"><rotation degrees="90"/></image></fix-orientation></filters>
</project>"#;

    #[test]
    fn page_list_and_rotation_are_read() {
        let p = parse(XML, None).unwrap();
        assert_eq!(p.sheets.len(), 2);
        assert_eq!(p.sheets[0].0, PathBuf::from("/scans/a.tif"));
        assert_eq!(p.sheets[1].0, PathBuf::from("/scans/b & c.tif"));
        assert_eq!(p.sheets[0].1.rotate_quarters, 0);
        assert_eq!(p.sheets[1].1.rotate_quarters, 1);
    }

    #[test]
    fn other_text_is_refused() {
        assert!(parse("legatorocr-project 1\n", None).is_err());
        assert!(parse("<?xml version=\"1.0\"?><project></project>", None).is_err());
    }
}
