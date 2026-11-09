//! DNS-SD names and records (RFC 6763) as pure functions: service-type and instance-name rules,
//! the browse / meta-query / subtype names, TXT attribute parsing and the assembly of PTR, SRV,
//! TXT and address answers into a list of instances.
//!
//! No packet is sent or received here. The multicast transport for the `local.` domain is a
//! different RFC and is not implemented; callers hand in the answers they received.

use std::collections::BTreeMap;
use std::net::IpAddr;

/// Longest service name (the part after `_`), in characters.
const MAX_SERVICE_NAME: usize = 15;
/// Longest DNS label, in bytes.
const MAX_LABEL: usize = 63;

/// Is `t` a service type such as `_something._tcp`?
pub fn valid_service_type(t: &str) -> bool {
    let Some((name, proto)) = t.rsplit_once('.') else {
        return false;
    };
    if !(proto.eq_ignore_ascii_case("_tcp") || proto.eq_ignore_ascii_case("_udp")) {
        return false;
    }
    let Some(n) = name.strip_prefix('_') else {
        return false;
    };
    let b = n.as_bytes();
    !b.is_empty()
        && b.len() <= MAX_SERVICE_NAME
        && b.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'-')
        && b[0].is_ascii_alphanumeric()
        && b[b.len() - 1].is_ascii_alphanumeric()
        && !n.contains("--")
        && b.iter().any(|c| c.is_ascii_alphabetic())
}

/// Why an instance name is refused, if it is. The name is a free-form UTF-8 label: at most 63
/// bytes and no ASCII control characters. (Unicode normalization form C is expected of the
/// sender and is not enforced here.)
pub fn instance_name_problem(n: &str) -> Option<&'static str> {
    if n.is_empty() {
        Some("empty name")
    } else if n.len() > MAX_LABEL {
        Some("longer than 63 bytes")
    } else if n.chars().any(|c| c.is_ascii_control()) {
        Some("contains a control character")
    } else {
        None
    }
}

fn domain_dot(d: &str) -> String {
    let d = if d.is_empty() { "local" } else { d };
    format!("{}.", d.trim_end_matches('.'))
}

/// Name to query with PTR to list the instances of a service.
pub fn browse_name(service: &str, domain: &str) -> String {
    format!("{service}.{}", domain_dot(domain))
}

/// Name to query with PTR to list the service types present in a domain.
pub fn meta_query_name(domain: &str) -> String {
    format!("_services._dns-sd._udp.{}", domain_dot(domain))
}

/// Name that narrows a browse to a subtype of a service.
pub fn subtype_name(subtype: &str, service: &str, domain: &str) -> String {
    format!("{subtype}._sub.{service}.{}", domain_dot(domain))
}

/// Full name of an instance: its SRV and TXT records carry this name.
pub fn instance_fqdn(instance: &str, service: &str, domain: &str) -> String {
    format!("{instance}.{service}.{}", domain_dot(domain))
}

/// TXT attributes. A key without `=` is a boolean (`None`); keys are compared without regard
/// to case and the first occurrence of a key wins.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Txt {
    pub attrs: Vec<(String, Option<Vec<u8>>)>,
    /// `txtvers` is present but is not the first attribute.
    pub txtvers_not_first: bool,
}

impl Txt {
    pub fn get(&self, key: &str) -> Option<&Option<Vec<u8>>> {
        self.attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }
}

/// Parse TXT record data: a series of strings, each preceded by its length byte. A record of
/// length zero, or a single zero byte, has no attributes; strings of length zero are skipped,
/// so do keys that are empty or hold characters outside printable ASCII.
pub fn parse_txt(rdata: &[u8]) -> Txt {
    let mut out = Txt::default();
    let mut i = 0;
    let mut n_strings = 0;
    while i < rdata.len() {
        let len = rdata[i] as usize;
        i += 1;
        let end = (i + len).min(rdata.len());
        let s = &rdata[i..end];
        i = end;
        if s.is_empty() {
            continue;
        }
        n_strings += 1;
        let (key, val) = match s.iter().position(|b| *b == b'=') {
            Some(p) => (&s[..p], Some(s[p + 1..].to_vec())),
            None => (s, None),
        };
        if key.is_empty() || !key.iter().all(|b| (0x20..=0x7e).contains(b)) {
            continue;
        }
        let key = String::from_utf8_lossy(key).into_owned();
        if out.attrs.iter().any(|(k, _)| k.eq_ignore_ascii_case(&key)) {
            continue;
        }
        if key.eq_ignore_ascii_case("txtvers") && n_strings > 1 {
            out.txtvers_not_first = true;
        }
        out.attrs.push((key, val));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Srv {
    /// Instance label the record belongs to.
    pub instance: String,
    pub priority: u16,
    pub weight: u16,
    pub port: u16,
    pub target: String,
}

/// What came back from the network for one service type.
#[derive(Debug, Clone, Default)]
pub struct Answers {
    /// Instance labels from the PTR records.
    pub instances: Vec<String>,
    pub srv: Vec<Srv>,
    /// (instance label, TXT record data)
    pub txt: Vec<(String, Vec<u8>)>,
    /// (host name, address)
    pub addrs: Vec<(String, IpAddr)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub addrs: Vec<IpAddr>,
    pub txt: Txt,
}

#[derive(Debug, Default)]
pub struct Listing {
    pub instances: Vec<Instance>,
    /// Instances left out, with the reason.
    pub rejected: Vec<(String, String)>,
    /// Set when `instances` is empty: nothing answered is not an error.
    pub reason: Option<String>,
}

/// Build the list of usable instances. An instance is listed once however many times it was
/// seen (names compare without regard to ASCII case) and its addresses are merged. Among its
/// SRV records the one with the lowest priority is used, the highest weight among those.
/// A name with a control character or longer than 63 bytes is rejected; so is an instance
/// whose chosen SRV has port 0 (a placeholder, not a service). A missing TXT means no attributes.
pub fn assemble(a: &Answers) -> Listing {
    let mut l = Listing::default();
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut names: Vec<&String> = a.instances.iter().collect();
    for s in &a.srv {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&s.instance)) {
            names.push(&s.instance);
        }
    }
    for name in names {
        let key = name.to_lowercase();
        if seen.insert(key.clone(), ()).is_some() {
            continue;
        }
        if let Some(why) = instance_name_problem(name) {
            l.rejected.push((name.clone(), why.to_string()));
            continue;
        }
        let best = a
            .srv
            .iter()
            .filter(|s| s.instance.eq_ignore_ascii_case(name))
            .min_by_key(|s| (s.priority, std::cmp::Reverse(s.weight)));
        let Some(srv) = best else {
            l.rejected.push((name.clone(), "no SRV record".into()));
            continue;
        };
        if srv.port == 0 {
            l.rejected.push((
                name.clone(),
                "port 0: a name placeholder, not a service".into(),
            ));
            continue;
        }
        let mut addrs: Vec<IpAddr> = Vec::new();
        for (h, ip) in &a.addrs {
            if h.trim_end_matches('.')
                .eq_ignore_ascii_case(srv.target.trim_end_matches('.'))
                && !addrs.contains(ip)
            {
                addrs.push(*ip);
            }
        }
        let txt = a
            .txt
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, d)| parse_txt(d))
            .unwrap_or_default();
        l.instances.push(Instance {
            name: name.clone(),
            host: srv.target.clone(),
            port: srv.port,
            addrs,
            txt,
        });
    }
    if l.instances.is_empty() {
        l.reason = Some(if l.rejected.is_empty() {
            "no instance answered in the time allowed".to_string()
        } else {
            "every answer was rejected".to_string()
        });
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt(parts: &[&[u8]]) -> Vec<u8> {
        let mut v = Vec::new();
        for p in parts {
            v.push(p.len() as u8);
            v.extend_from_slice(p);
        }
        v
    }

    #[test]
    fn service_types() {
        assert!(valid_service_type("_http._tcp"));
        assert!(valid_service_type("_HTTP._UDP"));
        for bad in [
            "_nume-prea-lung-de-15._tcp",
            "_a--b._tcp",
            "_123._tcp",
            "_-a._tcp",
            "_a-._tcp",
            "http._tcp",
            "_http._sctp",
            "_._tcp",
            "_http",
        ] {
            assert!(!valid_service_type(bad), "{bad}");
        }
        assert!(!valid_service_type("_123456789012345a._tcp"));
        assert!(valid_service_type("_12345678901234a._tcp"));
    }

    #[test]
    fn instance_names_by_bytes_and_controls() {
        assert_eq!(instance_name_problem(&"a".repeat(63)), None);
        assert!(instance_name_problem(&"a".repeat(64)).is_some());
        // 32 two-byte characters = 64 bytes
        assert!(instance_name_problem(&"é".repeat(32)).is_some());
        assert_eq!(instance_name_problem(&"é".repeat(31)), None);
        assert!(instance_name_problem("bad\u{7}name").is_some());
        assert_eq!(instance_name_problem("Birou, etaj 2 \u{1F5A8}"), None);
        assert!(instance_name_problem("").is_some());
    }

    #[test]
    fn names_of_queries() {
        assert_eq!(meta_query_name("local."), "_services._dns-sd._udp.local.");
        assert_eq!(meta_query_name(""), "_services._dns-sd._udp.local.");
        assert_eq!(browse_name("_x._tcp", "local"), "_x._tcp.local.");
        assert_eq!(
            subtype_name("_color", "_x._tcp", "local."),
            "_color._sub._x._tcp.local."
        );
        assert_eq!(
            instance_fqdn("Mine", "_x._tcp", "local."),
            "Mine._x._tcp.local."
        );
    }

    #[test]
    fn txt_attributes() {
        let t = parse_txt(&txt(&[b"a=1", b"b", b"A=2"]));
        assert_eq!(t.get("a"), Some(&Some(b"1".to_vec())));
        assert_eq!(t.get("B"), Some(&None));
        assert_eq!(t.attrs.len(), 2);
        let t = parse_txt(&txt(&[b"k=a=b", b"e="]));
        assert_eq!(t.get("k"), Some(&Some(b"a=b".to_vec())));
        assert_eq!(t.get("e"), Some(&Some(Vec::new())));
        // a single zero byte, an empty record and no record are the same
        assert_eq!(parse_txt(&[0]), Txt::default());
        assert_eq!(parse_txt(&[]), Txt::default());
        // empty or non-printable keys are skipped; a short buffer does not panic
        let t = parse_txt(&txt(&[b"=x", b"\x01k=v", b"ok=1"]));
        assert_eq!(t.attrs.len(), 1);
        let _ = parse_txt(&[200, b'a']);
        // txtvers should come first
        assert!(!parse_txt(&txt(&[b"txtvers=1", b"a=1"])).txtvers_not_first);
        assert!(parse_txt(&txt(&[b"a=1", b"txtvers=1"])).txtvers_not_first);
    }

    fn srv(inst: &str, prio: u16, weight: u16, port: u16, target: &str) -> Srv {
        Srv {
            instance: inst.into(),
            priority: prio,
            weight,
            port,
            target: target.into(),
        }
    }

    #[test]
    fn lowest_priority_then_highest_weight() {
        let a = Answers {
            instances: vec!["Scan".into()],
            srv: vec![
                srv("Scan", 20, 5, 9000, "far.local."),
                srv("Scan", 10, 1, 8000, "near.local."),
                srv("Scan", 10, 7, 8001, "nearer.local."),
            ],
            ..Answers::default()
        };
        let l = assemble(&a);
        assert_eq!(l.instances.len(), 1);
        assert_eq!(
            (l.instances[0].port, l.instances[0].host.as_str()),
            (8001, "nearer.local.")
        );
    }

    #[test]
    fn port_zero_missing_txt_and_duplicates() {
        let a = Answers {
            instances: vec!["A".into(), "a".into(), "B".into(), "C".into()],
            srv: vec![
                srv("A", 0, 0, 80, "h.local."),
                srv("B", 0, 0, 0, "h.local."),
                srv("C", 0, 0, 81, "h2.local."),
            ],
            txt: vec![("A".into(), vec![0])],
            addrs: vec![
                ("h.local.".into(), "192.0.2.1".parse().unwrap()),
                ("H.local".into(), "192.0.2.1".parse().unwrap()),
                ("h.local.".into(), "2001:db8::1".parse().unwrap()),
            ],
        };
        let l = assemble(&a);
        let names: Vec<_> = l.instances.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["A", "C"]);
        assert_eq!(l.instances[0].addrs.len(), 2, "same address once");
        assert!(l.instances[0].txt.attrs.is_empty());
        assert!(l.instances[1].txt.attrs.is_empty() && l.instances[1].addrs.is_empty());
        assert_eq!(l.rejected.len(), 1);
        assert!(l.rejected[0].1.contains("port 0"));
        assert!(l.reason.is_none());
    }

    #[test]
    fn nothing_answered_is_an_empty_list_with_a_reason_not_an_error() {
        let l = assemble(&Answers::default());
        assert!(l.instances.is_empty());
        assert!(l.reason.as_deref().unwrap().contains("no instance"));
        let l = assemble(&Answers {
            instances: vec!["x\u{1}".into()],
            ..Answers::default()
        });
        assert!(l.instances.is_empty() && l.rejected.len() == 1);
    }
}
