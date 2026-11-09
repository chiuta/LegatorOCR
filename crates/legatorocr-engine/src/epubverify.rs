//! Structure and accessibility-metadata check of an EPUB 3 file (`--verify-epub`): container,
//! package document, navigation document, XML well-formedness of the XML resources, media
//! types and fallback chains, resource locations, accessibility discovery metadata, the
//! conformance claim and page navigation. It reports findings; it does not replace a full
//! validator and does not look at the content for accessibility problems.

use crate::xmlmini::{self, Doc, Node};
use crate::zipmini;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Finding {
    pub level: Level,
    /// Place in the archive (member name, or `metadata`, `manifest` ...).
    pub place: String,
    /// The requirement that is not met, with the standard section where it comes from.
    pub requirement: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Conformance {
    /// No `dcterms:conformsTo`: nothing is claimed.
    NotDeclared,
    /// The claim as written (accepted).
    Declared(String),
    /// The value `none`: declared exception.
    Exception,
    /// A claim in the wrong form or below the minimum versions.
    Rejected(String),
}

#[derive(Debug)]
pub struct Report {
    pub findings: Vec<Finding>,
    pub conformance: Conformance,
}

impl Report {
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|f| f.level == Level::Error)
            .count()
    }
}

const DC: &str = "http://purl.org/dc/elements/1.1/";

struct Cx {
    findings: Vec<Finding>,
}

impl Cx {
    fn add(&mut self, level: Level, place: &str, req: impl Into<String>) {
        self.findings.push(Finding {
            level,
            place: place.to_string(),
            requirement: req.into(),
        });
    }
    fn err(&mut self, place: &str, req: impl Into<String>) {
        self.add(Level::Error, place, req);
    }
    fn warn(&mut self, place: &str, req: impl Into<String>) {
        self.add(Level::Warning, place, req);
    }

    /// Parse an XML member and report what the EPUB rules forbid.
    fn xml(&mut self, place: &str, bytes: &[u8]) -> Option<Doc> {
        let doc = match xmlmini::parse_bytes(bytes) {
            Ok(d) => d,
            Err(e) => {
                self.err(
                    place,
                    format!("XML resources must be well-formed XML 1.0 (EPUB 3.3, 3.9): {e}"),
                );
                return None;
            }
        };
        if doc.external_entities || doc.external_subset {
            self.err(
                place,
                "no external entities or external subset in the DTD (EPUB 3.3, 3.9)",
            );
        }
        if doc.xinclude {
            self.err(place, "XInclude is not allowed (EPUB 3.3, 3.9)");
        }
        if let Some(enc) = &doc.encoding {
            let e = enc.to_ascii_lowercase().replace('_', "-");
            if !matches!(e.as_str(), "utf-8" | "utf8" | "utf-16" | "utf16") {
                self.err(
                    place,
                    format!("encoding must be UTF-8 or UTF-16, declared {enc} (EPUB 3.3, 3.9)"),
                );
            }
        }
        Some(doc)
    }
}

/// Media types that need no alternative (EPUB 3.3, 3.2): the list of core media types.
fn is_core(mt: &str) -> bool {
    let base = mt
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "image/gif"
            | "image/jpeg"
            | "image/png"
            | "image/svg+xml"
            | "image/webp"
            | "audio/mpeg"
            | "audio/mp4"
            | "audio/ogg"
            | "audio/opus"
            | "text/css"
            | "font/ttf"
            | "font/otf"
            | "font/sfnt"
            | "font/woff"
            | "font/woff2"
            | "application/font-sfnt"
            | "application/font-woff"
            | "application/vnd.ms-opentype"
            | "application/xhtml+xml"
            | "text/javascript"
            | "application/javascript"
            | "application/x-dtbncx+xml"
            | "application/smil+xml"
    )
}

fn is_content_doc(mt: &str) -> bool {
    matches!(
        mt.split(';').next().unwrap_or("").trim(),
        "application/xhtml+xml" | "image/svg+xml"
    )
}

fn is_xml_type(mt: &str) -> bool {
    matches!(
        mt.split(';').next().unwrap_or("").trim(),
        "application/xhtml+xml"
            | "image/svg+xml"
            | "application/x-dtbncx+xml"
            | "application/smil+xml"
    )
}

/// Audio, video and fonts may live outside the container (EPUB 3.3, 3.6).
fn may_be_remote(mt: &str) -> bool {
    mt.starts_with("audio/")
        || mt.starts_with("video/")
        || mt.starts_with("font/")
        || mt.contains("font")
        || mt.contains("opentype")
}

struct Item {
    id: String,
    href: String,
    media_type: String,
    fallback: Option<String>,
    properties: Vec<String>,
}

fn meta_values<'a>(meta: &'a [&'a Node], prop: &str) -> Vec<&'a str> {
    meta.iter()
        .filter(|m| m.attr("property") == Some(prop))
        .map(|m| m.text.trim())
        .collect()
}

fn version(t: &str) -> Option<(u32, u32)> {
    let (a, b) = t.split_once('.')?;
    Some((a.parse().ok()?, b.parse().ok()?))
}

/// `EPUB Accessibility <M.N> - WCAG <M.N> Level <A|AA|AAA>`; minimum 1.1 and 2.0.
fn conformance_claim(v: &str) -> Result<(), String> {
    let rest = v
        .strip_prefix("EPUB Accessibility ")
        .ok_or("must start with \"EPUB Accessibility \"")?;
    let (a11y, rest) = rest.split_once(" - WCAG ").ok_or("missing \" - WCAG \"")?;
    let (wcag, level) = rest.split_once(" Level ").ok_or("missing \" Level \"")?;
    if !matches!(level, "A" | "AA" | "AAA") {
        return Err(format!("level {level:?} is not A, AA or AAA"));
    }
    let a = version(a11y).ok_or("the accessibility version is not M.N")?;
    let w = version(wcag).ok_or("the WCAG version is not M.N")?;
    if a < (1, 1) {
        return Err(format!("EPUB Accessibility {a11y} is below 1.1"));
    }
    if w < (2, 0) {
        return Err(format!("WCAG {wcag} is below 2.0"));
    }
    Ok(())
}

fn resolve(base_dir: &str, href: &str) -> String {
    let path = href.split('#').next().unwrap_or("");
    let mut parts: Vec<&str> = if base_dir.is_empty() {
        Vec::new()
    } else {
        base_dir.split('/').collect()
    };
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn has_type(n: &Node, token: &str) -> bool {
    n.attrs.iter().any(|(k, v)| {
        (k == "epub:type" || k.ends_with(":type")) && v.split_whitespace().any(|t| t == token)
    })
}

pub fn check(data: &[u8]) -> Report {
    let mut cx = Cx {
        findings: Vec::new(),
    };
    let mut conformance = Conformance::NotDeclared;
    let Some(list) = zipmini::list(data) else {
        cx.err("archive", "an EPUB is one ZIP (OCF) file (EPUB 3.3, 4.1)");
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let members: BTreeMap<&str, &zipmini::Entry> =
        list.iter().map(|e| (e.name.as_str(), e)).collect();
    let read =
        |name: &str| -> Option<Vec<u8>> { members.get(name).and_then(|e| zipmini::read(data, e)) };

    if !members.contains_key("mimetype") {
        cx.err(
            "mimetype",
            "the container holds a file named mimetype (EPUB 3.3, 4.1)",
        );
    }
    let Some(container_bytes) = read("META-INF/container.xml") else {
        cx.err(
            "META-INF/container.xml",
            "the container holds META-INF/container.xml pointing to the package document (EPUB 3.3, 4.1)",
        );
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let Some(container) = cx.xml("META-INF/container.xml", &container_bytes) else {
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let root_path = container
        .root
        .all("rootfile")
        .first()
        .and_then(|r| r.attr("full-path"))
        .map(str::to_string);
    let Some(opf_path) = root_path else {
        cx.err(
            "META-INF/container.xml",
            "container.xml names the package document in a rootfile (EPUB 3.3, 4.1)",
        );
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let Some(opf_bytes) = read(&opf_path) else {
        cx.err(
            &opf_path,
            "the package document named in container.xml is in the container (EPUB 3.3, 4.1)",
        );
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let Some(opf) = cx.xml(&opf_path, &opf_bytes) else {
        return Report {
            findings: cx.findings,
            conformance,
        };
    };
    let opf_dir = opf_path
        .rsplit_once('/')
        .map(|x| x.0)
        .unwrap_or("")
        .to_string();
    let pkg = &opf.root;
    if pkg.local != "package" {
        cx.err(
            &opf_path,
            "the package document has a package root element (EPUB 3.3, 5)",
        );
    }
    if pkg.attr("unique-identifier").is_none() {
        cx.err(
            &opf_path,
            "package has a unique-identifier attribute (EPUB 3.3, 5.3)",
        );
    }

    // metadata
    let metadata = pkg.all("metadata").into_iter().next();
    let mut meta_nodes: Vec<&Node> = Vec::new();
    if let Some(m) = metadata {
        for term in ["identifier", "title", "language"] {
            let found = m.children.iter().any(|c| {
                c.local == term && c.ns.as_deref() == Some(DC) && !c.text.trim().is_empty()
            });
            if !found {
                cx.err(
                    &opf_path,
                    format!("metadata has a non-empty dc:{term} (EPUB 3.3, 5.5.3)"),
                );
            }
        }
        meta_nodes = m.children.iter().filter(|c| c.local == "meta").collect();
    } else {
        cx.err(
            &opf_path,
            "the package document has a metadata section (EPUB 3.3, 5.5)",
        );
    }

    // manifest and spine
    let mut items: Vec<Item> = Vec::new();
    for it in pkg.all("item") {
        items.push(Item {
            id: it.attr("id").unwrap_or("").to_string(),
            href: it.attr("href").unwrap_or("").to_string(),
            media_type: it.attr("media-type").unwrap_or("").to_string(),
            fallback: it.attr("fallback").map(str::to_string),
            properties: it
                .attr("properties")
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect(),
        });
    }
    if items.is_empty() {
        cx.err(
            &opf_path,
            "the manifest lists the resources of the publication (EPUB 3.3, 5.6)",
        );
    }
    let by_id: BTreeMap<&str, &Item> = items.iter().map(|i| (i.id.as_str(), i)).collect();
    let spine: Vec<&str> = pkg
        .all("itemref")
        .iter()
        .filter_map(|r| r.attr("idref"))
        .collect();
    if spine.is_empty() {
        cx.err(
            &opf_path,
            "the spine has at least one itemref (EPUB 3.3, 5.7)",
        );
    }
    for idref in &spine {
        if !by_id.contains_key(idref) {
            cx.err(
                &opf_path,
                format!("spine item {idref:?} is declared in the manifest (EPUB 3.3, 5.7)"),
            );
        }
    }

    // resource locations, existence, XML resources
    let mut nav_items: Vec<&Item> = Vec::new();
    for it in &items {
        let h = it.href.as_str();
        let mt = it.media_type.as_str();
        if it.properties.iter().any(|p| p == "nav") {
            nav_items.push(it);
        }
        if h.starts_with("data:") {
            cx.err(
                &opf_path,
                format!("manifest href {h:?}: data: URLs are not allowed (EPUB 3.3, 3.8)"),
            );
            continue;
        }
        if h.starts_with("file:") {
            cx.err(
                &opf_path,
                format!("manifest href {h:?}: file: URLs are not allowed (EPUB 3.3, 3.8)"),
            );
            continue;
        }
        if h.starts_with("http://") || h.starts_with("https://") {
            if !may_be_remote(mt) {
                cx.err(&opf_path, format!("{h}: only audio, video and fonts may be remote; other resources stay in the container (EPUB 3.3, 3.6)"));
            } else if h.starts_with("http://") {
                cx.warn(
                    &opf_path,
                    format!("{h}: remote resources should use https (EPUB 3.3, 3.7)"),
                );
            }
            continue;
        }
        let path = resolve(&opf_dir, &percent_decode(h));
        match read(&path) {
            None => cx.err(
                &opf_path,
                format!("manifest item {path:?} is a member of the container (EPUB 3.3, 3.6)"),
            ),
            Some(b) => {
                if is_xml_type(mt) {
                    cx.xml(&path, &b);
                }
            }
        }
    }

    // media types and fallback chains
    for it in &items {
        if is_core(&it.media_type) {
            continue;
        }
        let in_spine = spine.contains(&it.id.as_str());
        let mut seen = BTreeSet::new();
        let mut cur = it;
        let mut ok = false;
        let mut reason = String::from("no fallback");
        loop {
            if !seen.insert(cur.id.clone()) {
                reason = format!("the chain from {:?} loops back to {:?}", it.id, cur.id);
                cx.err(
                    &opf_path,
                    format!("fallback chains do not loop: {reason} (EPUB 3.3, 3.5)"),
                );
                reason.clear();
                break;
            }
            let Some(fb) = &cur.fallback else { break };
            match by_id.get(fb.as_str()) {
                None => {
                    reason = format!("fallback {fb:?} is not in the manifest");
                    break;
                }
                Some(next) => {
                    cur = next;
                    let reached = if in_spine {
                        is_content_doc(&cur.media_type)
                    } else {
                        is_core(&cur.media_type)
                    };
                    if reached {
                        ok = true;
                        break;
                    }
                }
            }
        }
        if !ok && !reason.is_empty() {
            let need = if in_spine {
                "a spine item of a foreign type has a fallback chain ending in an EPUB content document"
            } else {
                "a foreign resource has a fallback chain ending in a core media type"
            };
            cx.err(
                &opf_path,
                format!(
                    "{:?} ({}): {need} ({reason}) (EPUB 3.3, 3.2-3.5)",
                    it.href, it.media_type
                ),
            );
        }
    }

    // navigation document and page navigation
    let mut pagelist_targets: Option<BTreeSet<String>> = None;
    if nav_items.is_empty() {
        cx.err(&opf_path, "the publication has a navigation document, a manifest item with the nav property (EPUB 3.3, 2)");
    }
    for n in &nav_items {
        let path = resolve(&opf_dir, &percent_decode(&n.href));
        if let Some(doc) = read(&path).and_then(|b| xmlmini::parse_bytes(&b).ok()) {
            let mut navs = doc.root.all("nav");
            navs.retain(|x| has_type(x, "page-list"));
            if let Some(pl) = navs.first() {
                let mut set = BTreeSet::new();
                for a in pl.all("a") {
                    if let Some(h) = a.attr("href") {
                        set.insert(h.rsplit_once('#').map(|x| x.1).unwrap_or("").to_string());
                    }
                }
                pagelist_targets = Some(set);
            }
        }
    }
    let mut marker_ids: Vec<String> = Vec::new();
    for idref in &spine {
        if let Some(it) = by_id.get(idref) {
            if it.media_type == "application/xhtml+xml" {
                let path = resolve(&opf_dir, &percent_decode(&it.href));
                if let Some(doc) = read(&path).and_then(|b| xmlmini::parse_bytes(&b).ok()) {
                    collect_markers(&doc.root, &mut marker_ids);
                }
            }
        }
    }
    let has_source = metadata
        .map(|m| {
            m.children.iter().any(|c| {
                c.local == "source" && c.ns.as_deref() == Some(DC) && !c.text.trim().is_empty()
            })
        })
        .unwrap_or(false);
    match (&pagelist_targets, marker_ids.is_empty()) {
        (None, false) => cx.err(
            &opf_path,
            "a publication with page break markers has a page-list in its navigation document (EPUB 3.3, EPUB Accessibility 1.1, 3.4.1)",
        ),
        (Some(t), false) => {
            let missing = marker_ids.iter().filter(|id| !t.contains(*id)).count();
            if missing > 0 {
                cx.warn(
                    &opf_path,
                    format!("the page-list links every page break marker; {missing} of {} are not linked (EPUB Accessibility 1.1, 3.4.1)", marker_ids.len()),
                );
            }
        }
        _ => {}
    }
    if (pagelist_targets.is_some() || !marker_ids.is_empty()) && !has_source {
        cx.warn(
            &opf_path,
            "the source of the pagination (the print edition) is declared in the metadata (EPUB Accessibility 1.1, 3.4.1)",
        );
    }

    // accessibility discovery metadata
    for (prop, level) in [
        ("accessMode", Level::Error),
        ("accessibilityFeature", Level::Error),
        ("accessibilityHazard", Level::Error),
        ("accessibilitySummary", Level::Warning),
        ("accessModeSufficient", Level::Warning),
    ] {
        let vals = meta_values(&meta_nodes, &format!("schema:{prop}"));
        if vals.iter().all(|v| v.is_empty()) {
            let req = format!(
                "schema:{prop} is given in the package metadata (EPUB Accessibility 1.1, 2.2-2.3)"
            );
            cx.add(level, "metadata", req);
        }
    }

    // conformance claim
    let claims = meta_values(&meta_nodes, "dcterms:conformsTo");
    if let Some(c) = claims.first() {
        if *c == "none" {
            conformance = Conformance::Exception;
        } else {
            match conformance_claim(c) {
                Ok(()) => {
                    conformance = Conformance::Declared(c.to_string());
                    if meta_values(&meta_nodes, "a11y:certifiedBy")
                        .iter()
                        .all(|v| v.is_empty())
                    {
                        cx.err(
                            "metadata",
                            "a conformance claim names the certifier, a11y:certifiedBy (EPUB Accessibility 1.1, 3.5.3.1)",
                        );
                    }
                }
                Err(e) => {
                    cx.err(
                        "metadata",
                        format!("dcterms:conformsTo must read \"EPUB Accessibility M.N - WCAG M.N Level A|AA|AAA\", at least 1.1 and 2.0: {e} (EPUB Accessibility 1.1, 3.5.2)"),
                    );
                    conformance = Conformance::Rejected(c.to_string());
                }
            }
        }
    }
    Report {
        findings: cx.findings,
        conformance,
    }
}

fn collect_markers(n: &Node, out: &mut Vec<String>) {
    if has_type(n, "pagebreak") {
        if let Some(id) = n.attr("id") {
            out.push(id.to_string());
        }
    }
    for c in &n.children {
        collect_markers(c, out);
    }
}

pub fn render(r: &Report) -> String {
    let mut s = String::new();
    for f in &r.findings {
        let l = if f.level == Level::Error {
            "ERROR"
        } else {
            "warning"
        };
        s.push_str(&format!("{l}: {}: {}\n", f.place, f.requirement));
    }
    s.push_str(&match &r.conformance {
        Conformance::NotDeclared => "conformance: none declared\n".to_string(),
        Conformance::Declared(c) => format!("conformance: declared, {c}\n"),
        Conformance::Exception => "conformance: declared exception (none)\n".to_string(),
        Conformance::Rejected(c) => format!("conformance: claim rejected, {c}\n"),
    });
    s.push_str(&format!(
        "{} error(s), {} warning(s)\n",
        r.errors(),
        r.findings.len() - r.errors()
    ));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER: &str = "<?xml version=\"1.0\"?><container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\"><rootfiles><rootfile full-path=\"OEBPS/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles></container>";
    const NAV: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body><nav epub:type=\"toc\"><ol><li><a href=\"c.xhtml\">T</a></li></ol></nav>__PL__</body></html>";
    const CONTENT: &str = "<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><body><p>hi</p>__MARK__</body></html>";

    struct Book {
        meta: String,
        manifest_extra: String,
        spine_extra: String,
        nav_pl: String,
        marks: String,
        content: String,
        extra_members: Vec<(String, Vec<u8>)>,
        container: Option<String>,
    }

    impl Book {
        fn good() -> Book {
            Book {
                meta: "<dc:identifier id=\"i\">urn:x:1</dc:identifier><dc:title>T</dc:title><dc:language>ro</dc:language>\
                       <meta property=\"schema:accessMode\">textual</meta><meta property=\"schema:accessibilityFeature\">tableOfContents</meta>\
                       <meta property=\"schema:accessibilityHazard\">none</meta><meta property=\"schema:accessibilitySummary\">Text only.</meta>\
                       <meta property=\"schema:accessModeSufficient\">textual</meta>"
                    .into(),
                manifest_extra: String::new(),
                spine_extra: String::new(),
                nav_pl: String::new(),
                marks: String::new(),
                content: CONTENT.into(),
                extra_members: Vec::new(),
                container: Some(CONTAINER.into()),
            }
        }
        fn bytes(&self) -> Vec<u8> {
            let opf = format!(
                "<?xml version=\"1.0\"?><package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"i\">\
                 <metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\">{}</metadata>\
                 <manifest><item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>\
                 <item id=\"c\" href=\"c.xhtml\" media-type=\"application/xhtml+xml\"/>{}</manifest>\
                 <spine><itemref idref=\"c\"/>{}</spine></package>",
                self.meta, self.manifest_extra, self.spine_extra
            );
            let nav = NAV.replace("__PL__", &self.nav_pl);
            let content = self.content.replace("__MARK__", &self.marks);
            let mut m: Vec<(&str, &[u8])> = vec![("mimetype", b"application/epub+zip")];
            if let Some(c) = &self.container {
                m.push(("META-INF/container.xml", c.as_bytes()));
            }
            m.push(("OEBPS/content.opf", opf.as_bytes()));
            m.push(("OEBPS/nav.xhtml", nav.as_bytes()));
            m.push(("OEBPS/c.xhtml", content.as_bytes()));
            for (n, b) in &self.extra_members {
                m.push((n.as_str(), b.as_slice()));
            }
            zipmini::write(&m)
        }
    }

    fn msgs(r: &Report, level: Level) -> Vec<String> {
        r.findings
            .iter()
            .filter(|f| f.level == level)
            .map(|f| format!("{}: {}", f.place, f.requirement))
            .collect()
    }

    #[test]
    fn a_complete_book_has_no_finding_and_no_claim() {
        let r = check(&Book::good().bytes());
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        assert_eq!(r.conformance, Conformance::NotDeclared);
    }

    #[test]
    fn missing_container_parts_are_container_errors() {
        let mut b = Book::good();
        b.container = None;
        let r = check(&b.bytes());
        assert!(msgs(&r, Level::Error)
            .iter()
            .any(|m| m.contains("container.xml")));
        assert!(!check(b"not a zip").findings.is_empty());
    }

    #[test]
    fn missing_title_language_identifier() {
        for (cut, term) in [
            ("<dc:title>T</dc:title>", "dc:title"),
            ("<dc:language>ro</dc:language>", "dc:language"),
            (
                "<dc:identifier id=\"i\">urn:x:1</dc:identifier>",
                "dc:identifier",
            ),
        ] {
            let mut b = Book::good();
            b.meta = b.meta.replace(cut, "");
            let r = check(&b.bytes());
            assert!(
                msgs(&r, Level::Error).iter().any(|m| m.contains(term)),
                "{term}: {:?}",
                r.findings
            );
        }
    }

    #[test]
    fn no_navigation_document() {
        let mut b = Book::good();
        b.content = b.content.clone();
        let bytes = b.bytes();
        // rebuild the same book with the nav property removed
        let list = zipmini::list(&bytes).unwrap();
        let opf = zipmini::read(
            &bytes,
            list.iter().find(|e| e.name == "OEBPS/content.opf").unwrap(),
        )
        .unwrap();
        let opf = String::from_utf8(opf)
            .unwrap()
            .replace(" properties=\"nav\"", "");
        let mut members: Vec<(String, Vec<u8>)> = Vec::new();
        for e in &list {
            let d = if e.name == "OEBPS/content.opf" {
                opf.clone().into_bytes()
            } else {
                zipmini::read(&bytes, e).unwrap()
            };
            members.push((e.name.clone(), d));
        }
        let refs: Vec<(&str, &[u8])> = members
            .iter()
            .map(|(n, d)| (n.as_str(), d.as_slice()))
            .collect();
        let r = check(&zipmini::write(&refs));
        assert!(
            msgs(&r, Level::Error)
                .iter()
                .any(|m| m.contains("navigation document")),
            "{:?}",
            r.findings
        );
    }

    #[test]
    fn content_xml_must_be_well_formed_without_external_entities() {
        let mut b = Book::good();
        b.content = "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</body></html>".into();
        assert!(msgs(&check(&b.bytes()), Level::Error)
            .iter()
            .any(|m| m.contains("well-formed")));
        let mut b = Book::good();
        b.content = "<!DOCTYPE html [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>x</p></body></html>".into();
        assert!(msgs(&check(&b.bytes()), Level::Error)
            .iter()
            .any(|m| m.contains("external entities")));
    }

    #[test]
    fn foreign_resource_needs_a_fallback_chain() {
        let mut b = Book::good();
        b.manifest_extra = "<item id=\"t\" href=\"scan.tif\" media-type=\"image/tiff\"/>".into();
        b.extra_members
            .push(("OEBPS/scan.tif".into(), b"II*".to_vec()));
        let r = check(&b.bytes());
        assert!(
            msgs(&r, Level::Error)
                .iter()
                .any(|m| m.contains("scan.tif")),
            "{:?}",
            r.findings
        );
        b.manifest_extra = "<item id=\"t\" href=\"scan.tif\" media-type=\"image/tiff\" fallback=\"p\"/><item id=\"p\" href=\"scan.png\" media-type=\"image/png\"/>".into();
        b.extra_members
            .push(("OEBPS/scan.png".into(), b"\x89PNG".to_vec()));
        let r = check(&b.bytes());
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        // a loop between two foreign items
        b.manifest_extra = "<item id=\"t\" href=\"scan.tif\" media-type=\"image/tiff\" fallback=\"u\"/><item id=\"u\" href=\"b.tif\" media-type=\"image/tiff\" fallback=\"t\"/>".into();
        b.extra_members
            .push(("OEBPS/b.tif".into(), b"II*".to_vec()));
        let r = check(&b.bytes());
        assert!(
            msgs(&r, Level::Error).iter().any(|m| m.contains("loop")),
            "{:?}",
            r.findings
        );
        // self reference
        b.manifest_extra =
            "<item id=\"t\" href=\"scan.tif\" media-type=\"image/tiff\" fallback=\"t\"/>".into();
        let r = check(&b.bytes());
        assert!(msgs(&r, Level::Error).iter().any(|m| m.contains("loop")));
    }

    #[test]
    fn locations_of_resources() {
        let mut b = Book::good();
        b.manifest_extra = "<item id=\"a\" href=\"http://example.org/a.mp3\" media-type=\"audio/mpeg\"/><item id=\"i\" href=\"https://example.org/i.png\" media-type=\"image/png\"/><item id=\"d\" href=\"data:image/png;base64,AA\" media-type=\"image/png\"/>".into();
        let r = check(&b.bytes());
        let e = msgs(&r, Level::Error);
        assert!(e.iter().any(|m| m.contains("i.png")), "{e:?}");
        assert!(e.iter().any(|m| m.contains("data:")));
        assert!(!e.iter().any(|m| m.contains("a.mp3")));
        assert!(msgs(&r, Level::Warning).iter().any(|m| m.contains("https")));
    }

    #[test]
    fn accessibility_discovery_metadata() {
        let mut b = Book::good();
        b.meta = b
            .meta
            .replace(
                "<meta property=\"schema:accessibilityHazard\">none</meta>",
                "",
            )
            .replace(
                "<meta property=\"schema:accessibilitySummary\">Text only.</meta>",
                "",
            );
        let r = check(&b.bytes());
        assert!(msgs(&r, Level::Error)
            .iter()
            .any(|m| m.contains("accessibilityHazard")));
        assert!(msgs(&r, Level::Warning)
            .iter()
            .any(|m| m.contains("accessibilitySummary")));
    }

    #[test]
    fn conformance_claims() {
        let claim = |v: &str, certifier: bool| {
            let mut b = Book::good();
            b.meta
                .push_str(&format!("<meta property=\"dcterms:conformsTo\">{v}</meta>"));
            if certifier {
                b.meta
                    .push_str("<meta property=\"a11y:certifiedBy\">Someone</meta>");
            }
            check(&b.bytes())
        };
        let ok = claim("EPUB Accessibility 1.1 - WCAG 2.2 Level AA", true);
        assert!(ok.findings.is_empty(), "{:?}", ok.findings);
        assert!(matches!(ok.conformance, Conformance::Declared(_)));
        let old = claim("EPUB Accessibility 1.0 - WCAG 2.2 Level AA", true);
        assert!(matches!(old.conformance, Conformance::Rejected(_)));
        assert_eq!(old.errors(), 1);
        assert_eq!(claim("none", false).conformance, Conformance::Exception);
        assert!(claim("none", false).findings.is_empty());
        assert!(matches!(
            claim("WCAG 2.2", true).conformance,
            Conformance::Rejected(_)
        ));
        assert!(matches!(
            claim("EPUB Accessibility 1.1 - WCAG 1.0 Level AA", true).conformance,
            Conformance::Rejected(_)
        ));
        assert!(matches!(
            claim("EPUB Accessibility 1.1 - WCAG 2.2 Level B", true).conformance,
            Conformance::Rejected(_)
        ));
        // a claim needs a certifier
        let no_cert = claim("EPUB Accessibility 1.1 - WCAG 2.2 Level AA", false);
        assert!(msgs(&no_cert, Level::Error)
            .iter()
            .any(|m| m.contains("certifiedBy")));
    }

    #[test]
    fn page_list_needs_a_declared_source() {
        let mut b = Book::good();
        b.marks = "<span epub:type=\"pagebreak\" id=\"p1\" role=\"doc-pagebreak\" aria-label=\"1\"></span>".into();
        b.nav_pl =
            "<nav epub:type=\"page-list\"><ol><li><a href=\"c.xhtml#p1\">1</a></li></ol></nav>"
                .into();
        let r = check(&b.bytes());
        assert_eq!(r.errors(), 0, "{:?}", r.findings);
        assert!(msgs(&r, Level::Warning)
            .iter()
            .any(|m| m.contains("source of the pagination")));
        b.meta
            .push_str("<dc:source>urn:isbn:9780000000000</dc:source>");
        let r = check(&b.bytes());
        assert!(r.findings.is_empty(), "{:?}", r.findings);
        // markers without any page-list
        b.nav_pl.clear();
        assert!(msgs(&check(&b.bytes()), Level::Error)
            .iter()
            .any(|m| m.contains("page-list")));
    }

    #[test]
    fn the_book_this_program_writes_is_read_back() {
        let pages = vec![vec![
            crate::docx::Block::Head("Cap".into()),
            crate::docx::Block::Para("text".into()),
        ]];
        let z = crate::epub::from_blocks(&pages, "Carte", "ro");
        let r = check(&z);
        let e = msgs(&r, Level::Error);
        // known gap: the writer does not give accessibilityHazard (value list not in the spec)
        assert_eq!(e.len(), 1, "{:?}", r.findings);
        assert!(e[0].contains("accessibilityHazard"));
        assert_eq!(r.conformance, Conformance::NotDeclared);
    }
}
