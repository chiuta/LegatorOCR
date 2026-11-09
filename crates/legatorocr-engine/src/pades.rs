//! PAdES long-term material (spec 09, partial): put validation values into the document
//! security store, add a document time stamp over the result, and report the level a file
//! reaches with the conformance problems found.
//!
//! Not done here (see docs/clean-room/specs/INTREBARI.md): fetching OCSP/CRL values, checking the
//! certificate chain or the revocation values, and the optional per-signature (VRI) entries.

use crate::sign::{body_of, children, der_trim, signer_info_children};
use anyhow::{bail, Result};
use legatorocr_pdf::reader::sign::{self, SigInfo};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone)]
pub struct Finding {
    pub severity: Severity,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct SigLevel {
    /// 1-based index among all signature dictionaries in file order of object number.
    pub index: usize,
    /// `Sig` or `DocTimeStamp`.
    pub kind: String,
    /// `B-B`, `B-T`, `B-LT` or `B-LTA` (empty for a document time stamp).
    pub level: String,
    pub findings: Vec<Finding>,
}

fn is_der(v: &[u8]) -> bool {
    der_trim(v).is_some_and(|d| d.len() == v.len())
}

/// Add certificates, OCSP responses and CRLs (raw DER) to the document security store of a
/// signed PDF. Refuses, leaving the file as it was, when there is no signature, no certificate
/// or no revocation value, or a value is not DER.
pub fn extend_lt(
    data: &[u8],
    certs: &[Vec<u8>],
    ocsps: &[Vec<u8>],
    crls: &[Vec<u8>],
) -> Result<Vec<u8>> {
    let sigs = sign::signatures(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    if !sigs.iter().any(|s| s.sub_filter != "ETSI.RFC3161") {
        bail!("the file has no signature to extend");
    }
    if certs.is_empty() {
        bail!("no certificate given (--dss-cert): the long-term level needs the signer's chain");
    }
    if ocsps.is_empty() && crls.is_empty() {
        bail!(
            "no revocation value given (--dss-ocsp or --dss-crl): a file without them would only look long-term; \
             this program does not fetch them"
        );
    }
    for (what, list) in [
        ("certificate", certs),
        ("OCSP response", ocsps),
        ("CRL", crls),
    ] {
        for (i, v) in list.iter().enumerate() {
            if !is_der(v) {
                bail!("{what} {} is not a single DER element", i + 1);
            }
        }
    }
    sign::add_dss(data, certs, ocsps, crls).map_err(|e| anyhow::anyhow!("{e}"))
}

/// The time-stamp token (a ContentInfo) of a `TimeStampResp`.
fn token_of_response(resp: &[u8]) -> Result<Vec<u8>> {
    let bad = || anyhow::anyhow!("unexpected time-stamp response structure");
    Ok(children(body_of(resp).ok_or_else(bad)?)
        .ok_or_else(bad)?
        .get(1)
        .ok_or_else(bad)?
        .1
        .clone())
}

/// Add a document time stamp as the last incremental update. `stamp` gets the SHA-256 of the
/// bytes to cover and returns a `TimeStampResp`. Needs a document security store already in the
/// file (run [`extend_lt`] first); fails before any output when the token does not fit.
pub fn add_document_timestamp(
    data: &[u8],
    stamp: &dyn Fn(&[u8; 32]) -> Result<Vec<u8>>,
) -> Result<Vec<u8>> {
    match sign::dss_info(data).map_err(|e| anyhow::anyhow!("{e}"))? {
        None => bail!("the file has no validation store (DSS): add the validation material first (--pades-lt)"),
        Some(d) if d.certs == 0 || d.ocsps + d.crls == 0 => {
            bail!("the validation store lacks certificates or revocation values")
        }
        Some(_) => {}
    }
    let prepared = sign::prepare_doc_timestamp(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let digest = crate::tsa::digest_of(&prepared.signed_data());
    let resp = stamp(&digest)?;
    let token = token_of_response(&resp)?;
    prepared.embed(&token).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Compare the imprint of a time-stamp token (`contents`, DER with zero padding) with the
/// SHA-256 of `signed`.
pub fn check_imprint(contents: &[u8], signed: &[u8]) -> std::result::Result<(), String> {
    use crate::tsa::read;
    let bad = || "the token is not a readable time-stamp token".to_string();
    let tok = der_trim(contents).ok_or_else(bad)?;
    let (_, ci, _) = read(tok).ok_or_else(bad)?;
    let (_, _oid, ci) = read(ci).ok_or_else(bad)?;
    let (_, expl, _) = read(ci).ok_or_else(bad)?;
    let (_, sd, _) = read(expl).ok_or_else(bad)?;
    let (_, _v, sd) = read(sd).ok_or_else(bad)?;
    let (_, _algs, sd) = read(sd).ok_or_else(bad)?;
    let (_, encap, _) = read(sd).ok_or_else(bad)?;
    let (_, _ct, encap) = read(encap).ok_or_else(bad)?;
    let (_, e0, _) = read(encap).ok_or_else(bad)?;
    let (_, octets, _) = read(e0).ok_or_else(bad)?;
    let (_, tst, _) = read(octets).ok_or_else(bad)?;
    let (_, _ver, t) = read(tst).ok_or_else(bad)?;
    let (_, _pol, t) = read(t).ok_or_else(bad)?;
    let (_, imprint, _) = read(t).ok_or_else(bad)?;
    let (_, alg, imp) = read(imprint).ok_or_else(bad)?;
    let (_, digest, _) = read(imp).ok_or_else(bad)?;
    const SHA256: [u8; 9] = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];
    let (_, oid, _) = read(alg).ok_or_else(bad)?;
    if oid != SHA256 {
        return Err("the time stamp uses a digest other than SHA-256: imprint not compared".into());
    }
    if digest == crate::tsa::digest_of(signed) {
        Ok(())
    } else {
        Err("the time-stamp imprint does not match the covered bytes".into())
    }
}

const OID_SIGNING_TIME: [u8; 9] = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x05];
const OID_MD5: [u8; 8] = [0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x02, 0x05];
const OID_SIG_TS_ATTR: [u8; 11] = [
    0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x02, 0x0E,
];

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn err(m: impl Into<String>) -> Finding {
    Finding {
        severity: Severity::Error,
        message: m.into(),
    }
}
fn warn(m: impl Into<String>) -> Finding {
    Finding {
        severity: Severity::Warning,
        message: m.into(),
    }
}

/// Report the level every signature reaches and what breaks the PAdES baseline rules.
pub fn analyse(data: &[u8]) -> Result<Vec<SigLevel>> {
    let sigs = sign::signatures(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let dss = sign::dss_info(data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let end_of = |s: &SigInfo| {
        s.byte_range
            .get(2)
            .zip(s.byte_range.get(3))
            .map(|(a, b)| a + b)
            .unwrap_or(0)
    };
    let stamps: Vec<&SigInfo> = sigs
        .iter()
        .filter(|s| s.sub_filter == "ETSI.RFC3161" || s.kind == "DocTimeStamp")
        .collect();
    let mut out = Vec::new();
    for (i, s) in sigs.iter().enumerate() {
        let mut f = Vec::new();
        let is_stamp = stamps.iter().any(|t| std::ptr::eq(*t, s));
        if is_stamp {
            for k in [
                "Cert",
                "Reference",
                "Changes",
                "R",
                "Prop_AuthTime",
                "Prop_AuthType",
            ] {
                if s.keys.iter().any(|x| x == k) {
                    f.push(err(format!(
                        "the document time stamp has the forbidden entry /{k}"
                    )));
                }
            }
            for k in ["Name", "M", "Location", "Reason", "ContactInfo"] {
                if s.keys.iter().any(|x| x == k) {
                    f.push(warn(format!(
                        "the document time stamp should not have /{k}"
                    )));
                }
            }
            if s.v.is_some_and(|v| v != 0) {
                f.push(err("the document time stamp has /V other than 0"));
            }
            if s.sub_filter != "ETSI.RFC3161" {
                f.push(err(
                    "the document time stamp's /SubFilter is not ETSI.RFC3161",
                ));
            }
            match s.signed_data(data) {
                Some(signed) => {
                    if let Err(m) = check_imprint(&s.contents, &signed) {
                        f.push(err(m));
                    }
                }
                None => f.push(err("the byte range does not fit the file")),
            }
            if end_of(s) != 0 && end_of(s) < data.len() {
                f.push(warn("bytes were added after this time stamp"));
            }
            out.push(SigLevel {
                index: i + 1,
                kind: "DocTimeStamp".into(),
                level: String::new(),
                findings: f,
            });
            continue;
        }
        // an ordinary signature
        if s.sub_filter != "ETSI.CAdES.detached" {
            f.push(warn(format!(
                "/SubFilter is {:?}, the baseline profile uses ETSI.CAdES.detached",
                s.sub_filter
            )));
        }
        if s.keys.iter().any(|x| x == "Cert") {
            f.push(err(
                "the signature dictionary has the forbidden entry /Cert",
            ));
        }
        let cms = der_trim(&s.contents);
        let mut sig_ts = false;
        match cms.and_then(signer_info_children) {
            Some(items) => {
                // digestAlgorithm is the third child
                if items.get(2).is_some_and(|(_, e)| contains(e, &OID_MD5)) {
                    f.push(err("the signature uses MD5, which is forbidden"));
                }
                if let Some((_, attrs)) = items.iter().find(|(t, _)| *t == 0xA0) {
                    if contains(attrs, &OID_SIGNING_TIME) {
                        f.push(err(
                            "the signature has the CMS signing-time attribute, which is forbidden (the claimed time belongs in /M)",
                        ));
                    }
                }
                if let Some((_, un)) = items.iter().find(|(t, _)| *t == 0xA1) {
                    sig_ts = contains(un, &OID_SIG_TS_ATTR);
                }
            }
            None => f.push(warn(
                "the signature value could not be parsed: attributes not checked",
            )),
        }
        let end = end_of(s);
        let later_stamp = stamps
            .iter()
            .any(|t| t.byte_range.get(1).is_some_and(|&b| b > end));
        let mut level = "B-B";
        if sig_ts || later_stamp {
            level = "B-T";
        }
        let dss_ok = dss
            .as_ref()
            .is_some_and(|d| d.certs > 0 && d.ocsps + d.crls > 0);
        if level == "B-T" && dss_ok {
            level = "B-LT";
        }
        if level == "B-LT" {
            if let Some(off) = dss.as_ref().and_then(|d| d.offset) {
                // a document time stamp that covers the store, which in turn comes after the signature
                let ordered = off >= end;
                if ordered
                    && stamps
                        .iter()
                        .any(|t| t.byte_range.get(1).is_some_and(|&b| b > off))
                {
                    level = "B-LTA";
                }
            }
        }
        if sigs.len() > 1 && level != "B-B" && !dss_ok && dss.is_some() {
            f.push(warn(
                "the validation store is incomplete for this signature",
            ));
        }
        out.push(SigLevel {
            index: i + 1,
            kind: "Sig".into(),
            level: level.into(),
            findings: f,
        });
    }
    Ok(out)
}

pub fn render(levels: &[SigLevel]) -> String {
    let mut s = String::new();
    if levels.is_empty() {
        s.push_str("no signature found\n");
    }
    for l in levels {
        if l.kind == "DocTimeStamp" {
            s.push_str(&format!("#{}: document time stamp\n", l.index));
        } else {
            s.push_str(&format!("#{}: signature, level {}\n", l.index, l.level));
        }
        for f in &l.findings {
            let tag = if f.severity == Severity::Error {
                "ERROR"
            } else {
                "warning"
            };
            s.push_str(&format!("  {tag}: {}\n", f.message));
        }
    }
    s.push_str("chain and revocation values were not validated\n");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sign::der_tlv;

    fn pdf() -> Vec<u8> {
        let mut w = legatorocr_pdf::PdfWriter::new(legatorocr_pdf::Compress::Flate);
        w.add_page(
            &legatorocr_core::Page {
                image: legatorocr_core::PageImage::Bw(legatorocr_core::BitImage::new(32, 32)),
                dpi: 72,
                skew_deg: 0.0,
                blank: false,
                split_index: None,
                content_box: None,
                layers: None,
            },
            &[],
        );
        w.finish()
    }

    /// A signed file whose CMS is a stand-in SEQUENCE (not a real signature).
    fn signed_stub() -> Vec<u8> {
        let p = sign::prepare(&pdf(), 1_782_864_000, Some("T"), None).unwrap();
        p.embed(&[0x30, 0x03, 0x02, 0x01, 0x00]).unwrap()
    }

    fn der(n: u8) -> Vec<u8> {
        der_tlv(0x30, &[0x02, 0x01, n])
    }

    /// A TimeStampResp holding a token whose imprint is `d`.
    fn fake_resp(d: &[u8; 32]) -> Vec<u8> {
        let alg = der_tlv(
            0x30,
            &[
                der_tlv(
                    0x06,
                    &[0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01],
                ),
                vec![0x05, 0x00],
            ]
            .concat(),
        );
        let imprint = der_tlv(0x30, &[alg, der_tlv(0x04, d)].concat());
        let tst = der_tlv(
            0x30,
            &[
                vec![0x02, 0x01, 0x01],
                der_tlv(0x06, &[0x2A, 0x03]),
                imprint,
                vec![0x02, 0x01, 0x05],
                der_tlv(0x18, b"20261109000000Z"),
            ]
            .concat(),
        );
        let encap = der_tlv(
            0x30,
            &[
                der_tlv(0x06, &[0x2A, 0x04]),
                der_tlv(0xA0, &der_tlv(0x04, &tst)),
            ]
            .concat(),
        );
        let sd = der_tlv(
            0x30,
            &[vec![0x02, 0x01, 0x03], der_tlv(0x31, &[]), encap].concat(),
        );
        let token = der_tlv(
            0x30,
            &[der_tlv(0x06, &[0x2A, 0x05]), der_tlv(0xA0, &sd)].concat(),
        );
        der_tlv(0x30, &[der_tlv(0x30, &[0x02, 0x01, 0x00]), token].concat())
    }

    fn lt_file() -> Vec<u8> {
        extend_lt(&signed_stub(), &[der(1)], &[der(2)], &[der(3)]).unwrap()
    }

    #[test]
    fn lt_is_an_incremental_update_with_streams() {
        let before = signed_stub();
        let after = lt_file();
        assert!(after.starts_with(&before));
        let d = sign::dss_info(&after).unwrap().unwrap();
        assert_eq!((d.certs, d.ocsps, d.crls, d.vri), (1, 1, 1, false));
        // the signature stays before the store
        let s = &sign::signatures(&after).unwrap()[0];
        assert!(d.offset.unwrap() >= s.byte_range[2] + s.byte_range[3]);
        // the streams hold the DER as given
        assert!(after
            .windows(5)
            .any(|w| w == der(2).as_slice().get(..5).unwrap()));
    }

    #[test]
    fn lt_refusals_leave_the_input_alone() {
        let f = signed_stub();
        assert!(extend_lt(&f, &[der(1)], &[], &[]).is_err());
        assert!(extend_lt(&f, &[], &[der(2)], &[]).is_err());
        assert!(extend_lt(&f, &[der(1)], &[vec![1, 2, 3]], &[]).is_err());
        assert!(extend_lt(&pdf(), &[der(1)], &[der(2)], &[]).is_err());
    }

    #[test]
    fn lta_adds_a_conforming_stamp_and_keeps_order() {
        let lt = lt_file();
        let lta = add_document_timestamp(&lt, &|d| Ok(fake_resp(d))).unwrap();
        assert!(lta.starts_with(&lt));
        let sigs = sign::signatures(&lta).unwrap();
        let ts = sigs.last().unwrap();
        assert_eq!(ts.kind, "DocTimeStamp");
        assert_eq!(ts.sub_filter, "ETSI.RFC3161");
        for k in [
            "Cert",
            "Reference",
            "Changes",
            "R",
            "Prop_AuthTime",
            "Prop_AuthType",
            "Name",
            "M",
            "Reason",
        ] {
            assert!(!ts.keys.iter().any(|x| x == k), "{k}");
        }
        assert!(ts.covers_whole_file(&lta));
        check_imprint(&ts.contents, &ts.signed_data(&lta).unwrap()).unwrap();
        let lv = analyse(&lta).unwrap();
        assert_eq!(lv[0].level, "B-LTA");
        assert!(lv[1].findings.is_empty(), "{:?}", lv[1].findings);
        // signature, store, stamp in this order
        let off = sign::dss_info(&lta).unwrap().unwrap().offset.unwrap();
        assert!(sigs[0].byte_range[2] <= off && off < ts.byte_range[1]);
    }

    #[test]
    fn lta_twice_keeps_old_values_and_adds_a_second_stamp() {
        let lta = add_document_timestamp(&lt_file(), &|d| Ok(fake_resp(d))).unwrap();
        let more = extend_lt(&lta, &[der(7)], &[der(8)], &[]).unwrap();
        let d = sign::dss_info(&more).unwrap().unwrap();
        assert_eq!((d.certs, d.ocsps, d.crls), (2, 2, 1));
        let again = add_document_timestamp(&more, &|d| Ok(fake_resp(d))).unwrap();
        let n = sign::signatures(&again)
            .unwrap()
            .iter()
            .filter(|s| s.kind == "DocTimeStamp")
            .count();
        assert_eq!(n, 2);
        let lv = analyse(&again).unwrap();
        assert!(lv.iter().filter(|l| l.kind == "DocTimeStamp").count() == 2);
    }

    #[test]
    fn lta_without_store_or_with_oversized_token_fails() {
        assert!(add_document_timestamp(&signed_stub(), &|d| Ok(fake_resp(d))).is_err());
        let lt = lt_file();
        let huge = |_: &[u8; 32]| {
            let token = der_tlv(0x30, &vec![0u8; 30000]);
            Ok(der_tlv(
                0x30,
                &[der_tlv(0x30, &[0x02, 0x01, 0x00]), token].concat(),
            ))
        };
        assert!(add_document_timestamp(&lt, &huge).is_err());
    }

    #[test]
    fn imprint_mismatch_is_an_error() {
        let lt = lt_file();
        let lta = add_document_timestamp(&lt, &|_| Ok(fake_resp(&[9u8; 32]))).unwrap();
        let lv = analyse(&lta).unwrap();
        assert!(lv[1].findings.iter().any(|f| f.severity == Severity::Error));
    }

    #[test]
    fn a_plain_signature_is_b_b() {
        let lv = analyse(&signed_stub()).unwrap();
        assert_eq!(lv[0].level, "B-B");
    }
    /// Needs OpenSSL 3; skipped without it. Signs with and without `-cades` and checks which
    /// signatures the conformance check rejects.
    #[test]
    fn signing_time_attribute_is_flagged() {
        use std::process::Command;
        let prog = std::env::var("LEGATOROCR_OPENSSL").unwrap_or_else(|_| "openssl".into());
        let run = |args: &[&str]| {
            Command::new(&prog)
                .args(args)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        let d = std::env::temp_dir().join(format!("lo-pades-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = |n: &str| d.join(n).to_string_lossy().into_owned();
        if !run(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "2",
            "-subj",
            "/CN=T",
            "-keyout",
            &p("k.pem"),
            "-out",
            &p("c.pem"),
        ]) {
            return;
        }
        let base = pdf();
        let mut res = Vec::new();
        for (name, extra) in [("plain", None), ("cades", Some("-cades"))] {
            let prepared = sign::prepare(&base, 1_782_864_000, Some("T"), None).unwrap();
            std::fs::write(d.join("in.bin"), prepared.signed_data()).unwrap();
            let (inp, cert, key) = (p("in.bin"), p("c.pem"), p("k.pem"));
            let out = p(&format!("{name}.der"));
            let mut args = vec![
                "cms", "-sign", "-binary", "-md", "sha256", "-outform", "DER", "-in", &inp,
                "-signer", &cert, "-inkey", &key, "-out", &out,
            ];
            if let Some(e) = extra {
                args.push(e);
            }
            if !run(&args) {
                let _ = std::fs::remove_dir_all(&d);
                return;
            }
            let signed = prepared.embed(&std::fs::read(&out).unwrap()).unwrap();
            let lv = analyse(&signed).unwrap();
            res.push((
                name,
                lv[0]
                    .findings
                    .iter()
                    .any(|f| f.message.contains("signing-time")),
            ));
        }
        let _ = std::fs::remove_dir_all(&d);
        // a CMS made without the baseline option carries the attribute and is flagged
        assert_eq!(res[0], ("plain", true));
        // record what the program's own signer option produces (reported, see the questions file)
        eprintln!("cades signing-time flagged: {}", res[1].1);
    }
}
