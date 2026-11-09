//! A small XML 1.0 reader for checking documents: it builds a tree with namespaces, reports
//! the first well-formedness error with its position, and notes what a hostile document would
//! use (external entities, XInclude). It is not a validating parser, resolves no external
//! resource and expands an internal entity only once (no entity inside an entity).

use std::collections::BTreeMap;

pub const XINCLUDE_NS: &str = "http://www.w3.org/2001/XInclude";

#[derive(Debug, Clone, Default)]
pub struct Node {
    /// Qualified name as written.
    pub name: String,
    pub local: String,
    pub ns: Option<String>,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Node>,
    /// Text directly inside this element, concatenated (entities decoded).
    pub text: String,
}

impl Node {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Descendants (not the node itself) in document order whose local name is `local`.
    pub fn find_all<'a>(&'a self, local: &str, out: &mut Vec<&'a Node>) {
        for c in &self.children {
            if c.local == local {
                out.push(c);
            }
            c.find_all(local, out);
        }
    }

    pub fn all<'a>(&'a self, local: &str) -> Vec<&'a Node> {
        let mut v = Vec::new();
        self.find_all(local, &mut v);
        v
    }

    /// All text below this element.
    pub fn deep_text(&self) -> String {
        let mut s = self.text.clone();
        for c in &self.children {
            s.push_str(&c.deep_text());
        }
        s
    }
}

#[derive(Debug, Clone, Default)]
pub struct Doc {
    pub root: Node,
    /// `<!ENTITY ... SYSTEM|PUBLIC ...>` in the internal subset, or an external subset.
    pub external_entities: bool,
    /// The document type declaration names an external subset.
    pub external_subset: bool,
    /// An element in the XInclude namespace.
    pub xinclude: bool,
    pub encoding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlError {
    pub message: String,
    pub offset: usize,
}

impl std::fmt::Display for XmlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (byte {})", self.message, self.offset)
    }
}

/// Deepest element nesting accepted (the tree walkers recurse once per level).
pub const MAX_DEPTH: usize = 256;
/// Most bytes all internal entity references of one document may expand to.
pub const MAX_ENTITY_EXPANSION: usize = 1 << 20;

struct P<'a> {
    s: &'a str,
    i: usize,
    entities: BTreeMap<String, String>,
    /// Bytes produced so far by internal entity references.
    expanded: usize,
    depth: usize,
    doc: Doc,
}

fn err<T>(p: &P, m: &str) -> Result<T, XmlError> {
    Err(XmlError {
        message: m.to_string(),
        offset: p.i,
    })
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == ':' || !c.is_ascii()
}

fn is_name(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '-' || c == '.'
}

impl<'a> P<'a> {
    fn rest(&self) -> &'a str {
        &self.s[self.i..]
    }
    fn starts(&self, t: &str) -> bool {
        self.rest().starts_with(t)
    }
    fn skip_ws(&mut self) {
        let t = self.rest().trim_start_matches([' ', '\t', '\r', '\n']);
        self.i = self.s.len() - t.len();
    }
    fn name(&mut self) -> Result<&'a str, XmlError> {
        let r = self.rest();
        let mut it = r.char_indices();
        match it.next() {
            Some((_, c)) if is_name_start(c) => {}
            _ => return err(self, "a name was expected"),
        }
        let end = r
            .char_indices()
            .find(|(_, c)| !is_name(*c))
            .map(|(k, _)| k)
            .unwrap_or(r.len());
        self.i += end;
        Ok(&r[..end])
    }
    fn until(&mut self, t: &str, what: &str) -> Result<&'a str, XmlError> {
        match self.rest().find(t) {
            Some(k) => {
                let r = &self.rest()[..k];
                self.i += k + t.len();
                Ok(r)
            }
            None => err(self, &format!("{what} is not closed")),
        }
    }

    fn decode(&mut self, raw: &str, in_attr: bool) -> Result<String, XmlError> {
        let mut out = String::new();
        let mut rest = raw;
        while let Some(k) = rest.find('&') {
            out.push_str(&rest[..k]);
            let after = &rest[k + 1..];
            let Some(e) = after.find(';') else {
                return err(self, "an & without ;");
            };
            let ent = &after[..e];
            match ent {
                "lt" => out.push('<'),
                "gt" => out.push('>'),
                "amp" => out.push('&'),
                "quot" => out.push('"'),
                "apos" => out.push('\''),
                _ if ent.starts_with('#') => {
                    let n = if let Some(h) = ent.strip_prefix("#x") {
                        u32::from_str_radix(h, 16).ok()
                    } else {
                        ent[1..].parse::<u32>().ok()
                    };
                    match n.and_then(char::from_u32) {
                        Some(c) if c == '\t' || c == '\n' || c == '\r' || c >= ' ' => out.push(c),
                        _ => return err(self, "a character reference is not a legal character"),
                    }
                }
                _ => match self.entities.get(ent).cloned() {
                    Some(v) => {
                        if v.contains('&') {
                            return err(self, "an entity inside an entity is not expanded");
                        }
                        if v.contains('<') && in_attr {
                            return err(self, "< in an attribute value");
                        }
                        self.expanded = self.expanded.saturating_add(v.len());
                        if self.expanded > MAX_ENTITY_EXPANSION {
                            return err(
                                self,
                                "entity references expand to more than 1 MiB in total",
                            );
                        }
                        out.push_str(&v);
                    }
                    None => return err(self, &format!("entity &{ent}; is not declared")),
                },
            }
            rest = &after[e + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    fn xml_decl(&mut self) -> Result<(), XmlError> {
        if !self.starts("<?xml") || !self.rest()[5..].starts_with([' ', '\t', '\r', '\n']) {
            return Ok(());
        }
        let body = self.until("?>", "the XML declaration")?;
        if let Some(k) = body.find("encoding") {
            let v = body[k + 8..]
                .trim_start()
                .trim_start_matches('=')
                .trim_start();
            if let Some(q) = v.chars().next().filter(|c| *c == '"' || *c == '\'') {
                if let Some(e) = v[1..].find(q) {
                    self.doc.encoding = Some(v[1..1 + e].to_string());
                }
            }
        }
        Ok(())
    }

    fn doctype(&mut self) -> Result<(), XmlError> {
        // "<!DOCTYPE" already seen
        self.i += 9;
        let start = self.i;
        let mut depth = 0i32;
        let mut quote: Option<char> = None;
        let mut end = None;
        for (k, c) in self.rest().char_indices() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), _) => {}
                (None, '"' | '\'') => quote = Some(c),
                (None, '[') => depth += 1,
                (None, ']') => depth -= 1,
                (None, '>') if depth <= 0 => {
                    end = Some(k);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return err(self, "the document type declaration is not closed");
        };
        let decl = &self.s[start..start + end];
        self.i = start + end + 1;
        let head = decl.split('[').next().unwrap_or("");
        if head
            .split_whitespace()
            .any(|w| w == "SYSTEM" || w == "PUBLIC")
        {
            self.doc.external_subset = true;
        }
        if let (Some(a), Some(b)) = (decl.find('['), decl.rfind(']')) {
            if a < b {
                let mut sub = &decl[a + 1..b];
                while let Some(k) = sub.find("<!ENTITY") {
                    let after = &sub[k + 8..];
                    let e = after.find('>').unwrap_or(after.len());
                    let d = after[..e].trim();
                    let parameter = d.starts_with('%');
                    let d = d.trim_start_matches('%').trim_start();
                    let name_end = d.find(char::is_whitespace).unwrap_or(d.len());
                    let (name, val) = (&d[..name_end], d[name_end..].trim());
                    if val.starts_with("SYSTEM") || val.starts_with("PUBLIC") {
                        self.doc.external_entities = true;
                    } else if !parameter && (val.starts_with('"') || val.starts_with('\'')) {
                        let q = &val[..1];
                        if let Some(close) = val[1..].find(q) {
                            self.entities
                                .insert(name.to_string(), val[1..1 + close].to_string());
                        }
                    }
                    sub = &after[e.min(after.len())..];
                }
            }
        }
        Ok(())
    }

    fn element(&mut self, scopes: &mut Vec<BTreeMap<String, String>>) -> Result<Node, XmlError> {
        // at '<'
        if self.depth >= MAX_DEPTH {
            return err(self, "elements are nested more than 256 levels deep");
        }
        self.depth += 1;
        let r = self.element_inner(scopes);
        self.depth -= 1;
        r
    }

    fn element_inner(
        &mut self,
        scopes: &mut Vec<BTreeMap<String, String>>,
    ) -> Result<Node, XmlError> {
        self.i += 1;
        let qname = self.name()?.to_string();
        let mut attrs: Vec<(String, String)> = Vec::new();
        loop {
            let before = self.i;
            self.skip_ws();
            if self.starts("/>") || self.starts(">") {
                break;
            }
            if self.i == before {
                return err(self, "a space was expected between attributes");
            }
            let an = self.name()?.to_string();
            self.skip_ws();
            if !self.starts("=") {
                return err(self, "= was expected after an attribute name");
            }
            self.i += 1;
            self.skip_ws();
            let q = self.rest().chars().next();
            let Some(q @ ('"' | '\'')) = q else {
                return err(self, "an attribute value must be quoted");
            };
            self.i += 1;
            let raw = self.until(&q.to_string(), "an attribute value")?;
            if raw.contains('<') {
                return err(self, "< in an attribute value");
            }
            if attrs.iter().any(|(k, _)| *k == an) {
                return err(self, &format!("attribute {an} given twice"));
            }
            let v = self.decode(raw, true)?;
            attrs.push((an, v));
        }
        let mut scope = BTreeMap::new();
        for (k, v) in &attrs {
            if k == "xmlns" {
                scope.insert(String::new(), v.clone());
            } else if let Some(p) = k.strip_prefix("xmlns:") {
                scope.insert(p.to_string(), v.clone());
            }
        }
        scopes.push(scope);
        let (prefix, local) = match qname.split_once(':') {
            Some((p, l)) => (p.to_string(), l.to_string()),
            None => (String::new(), qname.clone()),
        };
        let ns = scopes.iter().rev().find_map(|s| s.get(&prefix)).cloned();
        if prefix.as_str() == "xml" {
            // predefined prefix
        } else if !prefix.is_empty() && ns.is_none() {
            return err(self, &format!("namespace prefix {prefix} is not declared"));
        }
        if ns.as_deref() == Some(XINCLUDE_NS) {
            self.doc.xinclude = true;
        }
        let mut node = Node {
            name: qname.clone(),
            local,
            ns,
            attrs,
            ..Node::default()
        };
        if self.starts("/>") {
            self.i += 2;
            scopes.pop();
            return Ok(node);
        }
        self.i += 1; // '>'
        loop {
            if self.i >= self.s.len() {
                return err(self, &format!("element {qname} is not closed"));
            }
            if self.starts("</") {
                self.i += 2;
                let close = self.name()?;
                if close != qname {
                    return err(self, &format!("</{close}> closes <{qname}>"));
                }
                self.skip_ws();
                if !self.starts(">") {
                    return err(self, "> was expected");
                }
                self.i += 1;
                break;
            } else if self.starts("<!--") {
                self.i += 4;
                let c = self.until("-->", "a comment")?;
                if c.contains("--") {
                    return err(self, "-- inside a comment");
                }
            } else if self.starts("<![CDATA[") {
                self.i += 9;
                let t = self.until("]]>", "a CDATA section")?;
                node.text.push_str(t);
            } else if self.starts("<?") {
                self.i += 2;
                self.until("?>", "a processing instruction")?;
            } else if self.starts("<") {
                let c = self.element(scopes)?;
                node.children.push(c);
            } else {
                let k = self.rest().find('<').unwrap_or(self.rest().len());
                let raw = &self.rest()[..k];
                if raw.contains("]]>") {
                    return err(self, "]]> in text");
                }
                let t = self.decode(raw, false)?;
                node.text.push_str(&t);
                self.i += k;
            }
        }
        scopes.pop();
        Ok(node)
    }
}

/// Decode bytes as UTF-8 or UTF-16 (byte order mark required for the latter) and parse them.
pub fn parse_bytes(b: &[u8]) -> Result<Doc, XmlError> {
    let text = if b.starts_with(&[0xff, 0xfe]) || b.starts_with(&[0xfe, 0xff]) {
        let le = b[0] == 0xff;
        let units: Vec<u16> = b[2..]
            .chunks_exact(2)
            .map(|c| {
                if le {
                    u16::from_le_bytes([c[0], c[1]])
                } else {
                    u16::from_be_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|_| XmlError {
            message: "invalid UTF-16".into(),
            offset: 0,
        })?
    } else {
        let b = b.strip_prefix(&[0xef, 0xbb, 0xbf][..]).unwrap_or(b);
        String::from_utf8(b.to_vec()).map_err(|e| XmlError {
            message: "not valid UTF-8".into(),
            offset: e.utf8_error().valid_up_to(),
        })?
    };
    parse(&text)
}

pub fn parse(s: &str) -> Result<Doc, XmlError> {
    let mut p = P {
        s,
        i: 0,
        entities: BTreeMap::new(),
        expanded: 0,
        depth: 0,
        doc: Doc::default(),
    };
    p.xml_decl()?;
    let mut root: Option<Node> = None;
    loop {
        p.skip_ws();
        if p.i >= s.len() {
            break;
        }
        if p.starts("<!--") {
            p.i += 4;
            p.until("-->", "a comment")?;
        } else if p.starts("<?") {
            p.i += 2;
            p.until("?>", "a processing instruction")?;
        } else if p.starts("<!DOCTYPE") {
            if root.is_some() {
                return err(&p, "a document type declaration after the root element");
            }
            p.doctype()?;
        } else if p.starts("<") && !p.starts("</") {
            if root.is_some() {
                return err(&p, "a second root element");
            }
            let mut scopes = Vec::new();
            root = Some(p.element(&mut scopes)?);
        } else {
            return err(&p, "text outside the root element");
        }
    }
    let Some(r) = root else {
        return err(&p, "no root element");
    };
    p.doc.root = r;
    Ok(p.doc)
}

#[cfg(test)]
mod tests {
    #[test]
    fn doctype_with_a_bracket_before_the_subset_is_not_a_panic() {
        for d in [
            "<!DOCTYPE a ][ ><a/>",
            "<!DOCTYPE a ]><a/>",
            "<!DOCTYPE a [ ><a/>",
        ] {
            let _ = parse(d);
        }
    }

    use super::*;

    #[test]
    fn reads_a_tree_with_namespaces_and_entities() {
        let d = parse(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><!-- c --><a xmlns=\"u:a\" xmlns:p=\"u:p\" k='v'>x&amp;y<p:b>&#65;<![CDATA[<z>]]></p:b><c/></a>",
        )
        .unwrap();
        assert_eq!(d.encoding.as_deref(), Some("UTF-8"));
        assert_eq!(d.root.ns.as_deref(), Some("u:a"));
        assert_eq!(d.root.text, "x&y");
        assert_eq!(d.root.attr("k"), Some("v"));
        let b = &d.root.children[0];
        assert_eq!(
            (b.local.as_str(), b.ns.as_deref(), b.text.as_str()),
            ("b", Some("u:p"), "A<z>")
        );
        assert!(!d.xinclude && !d.external_entities);
    }

    #[test]
    fn malformed_documents_are_refused() {
        for bad in [
            "<a><b></a></b>",
            "<a>",
            "<a x=1/>",
            "<a x='1' x='2'/>",
            "<a>&nope;</a>",
            "<a/><b/>",
            "<p:a/>",
            "text",
            "<a>1 < 2</a>",
            "",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn external_entities_xinclude_and_internal_entities() {
        let d = parse("<!DOCTYPE a [<!ENTITY e SYSTEM \"file:///etc/passwd\">]><a>&e;</a>");
        // the reference to an undeclared (external) entity is not expanded: an error, but the
        // declaration is what a checker looks for
        assert!(d.is_err());
        let d = parse("<!DOCTYPE a [<!ENTITY e SYSTEM \"x\">]><a/>").unwrap();
        assert!(d.external_entities);
        let d = parse("<!DOCTYPE a SYSTEM \"a.dtd\"><a/>").unwrap();
        assert!(d.external_subset);
        let d = parse("<!DOCTYPE a [<!ENTITY e \"hi\">]><a>&e;</a>").unwrap();
        assert_eq!(d.root.text, "hi");
        let d =
            parse("<a xmlns:xi=\"http://www.w3.org/2001/XInclude\"><xi:include href=\"x\"/></a>")
                .unwrap();
        assert!(d.xinclude);
    }

    #[test]
    fn utf16_with_a_byte_order_mark_is_read() {
        let mut b = vec![0xff, 0xfe];
        for u in "<a>é</a>".encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_bytes(&b).unwrap().root.text, "é");
        assert!(parse_bytes(&[b'<', b'a', b'>', 0xff, b'<', b'/', b'a', b'>']).is_err());
    }
}
