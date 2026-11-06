//! PDF signatures (roadmap 5.7): sign an existing PDF with the user's own PKCS#12 certificate
//! (CAdES detached, SHA-256, as an incremental update so earlier content stays byte-identical),
//! and check the signatures of a PDF. The PDF structure comes from `legatorocr_pdf::reader::sign`;
//! the cryptography is delegated to the `openssl` command line tool.
//!
//! The legal weight of the result (simple / advanced / qualified under eIDAS) is that of the
//! certificate the user supplies; this program does not issue or validate trust anchors beyond
//! what `openssl cms -verify` does with the CA file given.

use anyhow::{bail, Context, Result};
use legatorocr_pdf::reader::sign;
use std::path::{Path, PathBuf};
use std::process::Command;

struct TempDir(PathBuf);
impl TempDir {
    fn new() -> Result<Self> {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let p = std::env::temp_dir().join(format!("lgr_sign_{}_{n}", std::process::id()));
        std::fs::create_dir(&p)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self(p))
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The `openssl` program that can make CAdES signatures (`cms -cades`, OpenSSL 3). The one on
/// the PATH is tried first; on macOS the system one is LibreSSL, which cannot, so the Homebrew
/// installs are tried next. `LEGATOROCR_OPENSSL` names one explicitly.
fn openssl_program() -> Option<&'static str> {
    use std::sync::OnceLock;
    static FOUND: OnceLock<Option<String>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let from_env = std::env::var("LEGATOROCR_OPENSSL").ok();
            let candidates = from_env.into_iter().chain(
                [
                    "openssl",
                    "/opt/homebrew/opt/openssl@3/bin/openssl",
                    "/usr/local/opt/openssl@3/bin/openssl",
                    "/opt/homebrew/bin/openssl",
                    "/usr/local/bin/openssl",
                ]
                .map(String::from),
            );
            for c in candidates {
                let out = Command::new(&c).args(["cms", "-help"]).output();
                if let Ok(o) = out {
                    let text = format!(
                        "{}{}",
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                    );
                    if text.contains("-cades") {
                        return Some(c);
                    }
                }
            }
            None
        })
        .as_deref()
}

fn openssl() -> Command {
    Command::new(openssl_program().unwrap_or("openssl"))
}

/// Sign `input` into `output`. The PKCS#12 password is read from the environment variable
/// `pass_env` (never from the command line).
pub fn sign_pdf(
    input: &Path,
    output: &Path,
    p12: &Path,
    pass_env: &str,
    name: Option<&str>,
    reason: Option<&str>,
    tsa_url: Option<&str>,
) -> Result<()> {
    let data = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    if openssl_program().is_none() {
        bail!(
            "signing needs OpenSSL 3 (`openssl cms -cades`); the openssl found is missing or too old \
             (macOS ships LibreSSL). Install OpenSSL 3 (macOS: `brew install openssl@3`) or point \
             LEGATOROCR_OPENSSL at it"
        );
    }
    if std::env::var_os(pass_env).is_none() {
        bail!("set the PKCS#12 password in the environment variable {pass_env} (use an empty value for none)");
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let prepared = sign::prepare(&data, now, name, reason).map_err(|e| anyhow::anyhow!("{e}"))?;
    let tmp = TempDir::new()?;
    let pem = tmp.0.join("key.pem");
    let st = openssl()
        .args(["pkcs12", "-in"])
        .arg(p12)
        .args(["-nodes", "-passin"])
        .arg(format!("env:{pass_env}"))
        .arg("-out")
        .arg(&pem)
        .output()
        .context("cannot run openssl")?;
    if !st.status.success() {
        bail!(
            "openssl could not open the PKCS#12 file (wrong password?): {}",
            String::from_utf8_lossy(&st.stderr).trim()
        );
    }
    // intermediate / root certificates of the file, if any (the signer's own is embedded already)
    let chain = tmp.0.join("chain.pem");
    let ok = openssl()
        .args(["pkcs12", "-in"])
        .arg(p12)
        .args(["-nokeys", "-cacerts", "-passin"])
        .arg(format!("env:{pass_env}"))
        .arg("-out")
        .arg(&chain)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    let has_chain = ok
        && std::fs::read_to_string(&chain)
            .map(|t| t.contains("BEGIN CERTIFICATE"))
            .unwrap_or(false);
    let content = tmp.0.join("content.bin");
    std::fs::write(&content, prepared.signed_data())?;
    let der = tmp.0.join("sig.der");
    let mut cmd = openssl();
    cmd.args([
        "cms", "-sign", "-binary", "-md", "sha256", "-cades", "-outform", "DER", "-in",
    ])
    .arg(&content)
    .arg("-signer")
    .arg(&pem)
    .arg("-inkey")
    .arg(&pem);
    if has_chain {
        cmd.arg("-certfile").arg(&chain);
    }
    let st = cmd
        .arg("-out")
        .arg(&der)
        .output()
        .context("cannot run openssl")?;
    if !st.status.success() {
        bail!(
            "openssl cms -sign failed: {}",
            String::from_utf8_lossy(&st.stderr).trim()
        );
    }
    let mut cms = std::fs::read(&der)?;
    if let Some(url) = tsa_url {
        // PAdES-B-T: a time stamp over the signature value, as an unsigned attribute
        let sig = signature_value(&cms).context("cannot find the signature value in the CMS")?;
        let mut d = [0u8; 32];
        d.copy_from_slice(&legatorocr_pdf::crypt::sha256(&sig));
        let (resp, _) = crate::tsa::request(url, &d)?;
        cms = add_timestamp(&cms, &resp)?;
    }
    let signed = prepared.embed(&cms).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(output, signed)?;
    Ok(())
}

// ---- minimal DER helpers for PAdES-B-T ----

fn der_len(n: usize) -> Vec<u8> {
    if n < 128 {
        vec![n as u8]
    } else if n < 256 {
        vec![0x81, n as u8]
    } else if n < 65536 {
        vec![0x82, (n >> 8) as u8, n as u8]
    } else {
        vec![0x83, (n >> 16) as u8, (n >> 8) as u8, n as u8]
    }
}

fn der_tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    v.extend(der_len(body.len()));
    v.extend_from_slice(body);
    v
}

/// Split the body of a constructed element into its (tag, whole element) children.
fn children(body: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let (tag, b, r) = crate::tsa::read(rest)?;
        let whole = &rest[..rest.len() - r.len()];
        let _ = b;
        out.push((tag, whole.to_vec()));
        rest = r;
    }
    Some(out)
}

fn body_of(el: &[u8]) -> Option<&[u8]> {
    crate::tsa::read(el).map(|(_, b, _)| b)
}

/// The path ContentInfo -> [0] -> SignedData -> signerInfos SET -> first SignerInfo, as the
/// children of the SignerInfo, plus the rebuilt-parent chain needed to put it back.
fn signer_info_children(cms: &[u8]) -> Option<Vec<(u8, Vec<u8>)>> {
    let ci = children(body_of(cms)?)?; // oid, [0]
    let sd = children(body_of(&ci.get(1)?.1)?)?; // SignedData
    let sd_items = children(body_of(&sd.first()?.1)?)?;
    let signer_infos = sd_items.last()?;
    let si = children(body_of(&signer_infos.1)?)?;
    children(body_of(&si.first()?.1)?)
}

/// The signature value (OCTET STRING) of the first signer.
pub fn signature_value(cms: &[u8]) -> Option<Vec<u8>> {
    let items = signer_info_children(cms)?;
    // version, sid, digestAlgorithm, [0] signedAttrs, signatureAlgorithm, signature
    let (_, el) = items.iter().find(|(t, _)| *t == 0x04)?;
    Some(body_of(el)?.to_vec())
}

/// Add the time-stamp token of the `TimeStampResp` `resp` to the first signer as the
/// unsigned attribute id-aa-signatureTimeStampToken.
pub fn add_timestamp(cms: &[u8], resp: &[u8]) -> Result<Vec<u8>> {
    let bad = || anyhow::anyhow!("unexpected CMS structure");
    let token = children(body_of(resp).ok_or_else(bad)?)
        .ok_or_else(bad)?
        .get(1)
        .ok_or_else(bad)?
        .1
        .clone();
    // Attribute ::= SEQUENCE { attrType OID, attrValues SET { token } }
    let oid = [
        0x06, 0x0B, 0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x02, 0x0E,
    ];
    let attr = der_tlv(0x30, &[oid.to_vec(), der_tlv(0x31, &token)].concat());
    let unsigned = der_tlv(0xA1, &attr);

    let ci = children(body_of(cms).ok_or_else(bad)?).ok_or_else(bad)?;
    let ctx0 = &ci.get(1).ok_or_else(bad)?.1;
    let sd = children(body_of(ctx0).ok_or_else(bad)?).ok_or_else(bad)?;
    let sd_items =
        children(body_of(&sd.first().ok_or_else(bad)?.1).ok_or_else(bad)?).ok_or_else(bad)?;
    let (set_tag, set) = sd_items.last().ok_or_else(bad)?.clone();
    let sis = children(body_of(&set).ok_or_else(bad)?).ok_or_else(bad)?;
    let (si_tag, si) = sis.first().ok_or_else(bad)?.clone();
    if sis.len() != 1 {
        bail!("only a single signer is supported");
    }
    let si_items = children(body_of(&si).ok_or_else(bad)?).ok_or_else(bad)?;
    if si_items.iter().any(|(t, _)| *t == 0xA1) {
        bail!("the signature already has unsigned attributes");
    }
    let new_si = der_tlv(si_tag, &[body_of(&si).ok_or_else(bad)?, &unsigned].concat());
    let new_set = der_tlv(set_tag, &new_si);
    let mut items: Vec<u8> = sd_items[..sd_items.len() - 1]
        .iter()
        .flat_map(|(_, e)| e.clone())
        .collect();
    items.extend(new_set);
    let new_sd = der_tlv(sd[0].0, &items);
    let new_ctx0 = der_tlv(0xA0, &new_sd);
    let new_ci = der_tlv(0x30, &[ci[0].1.clone(), new_ctx0].concat());
    Ok(new_ci)
}

pub struct SigReport {
    pub index: usize,
    pub name: Option<String>,
    pub sub_filter: String,
    pub covers_whole_file: bool,
    /// `Ok(())` when openssl accepted the signature.
    pub result: std::result::Result<(), String>,
    /// Time stamp inside the signature: `None` when there is none, else the time
    /// (`Ok`) or why it is not good (`Err`).
    pub timestamp: Option<std::result::Result<String, String>>,
}

/// The time-stamp token inside the first signer's unsigned attributes, if any.
fn embedded_token(cms: &[u8]) -> Option<Vec<u8>> {
    let items = signer_info_children(cms)?;
    let (_, unsigned) = items.iter().find(|(t, _)| *t == 0xA1)?;
    for (_, attr) in children(body_of(unsigned)?)? {
        let parts = children(body_of(&attr)?)?;
        if body_of(&parts.first()?.1)?
            == [
                0x2A, 0x86, 0x48, 0x86, 0xF7, 0x0D, 0x01, 0x09, 0x10, 0x02, 0x0E,
            ]
        {
            let vals = children(body_of(&parts.get(1)?.1)?)?;
            return Some(vals.first()?.1.clone());
        }
    }
    None
}

/// Check the time stamp inside `cms` (None when there is none): it must be for exactly the
/// signature value; with `tsa_ca` OpenSSL also checks the authority's signature.
fn check_embedded_timestamp(
    cms: &[u8],
    tsa_ca: Option<&Path>,
    tmp: &Path,
    i: usize,
) -> Option<std::result::Result<String, String>> {
    let token = embedded_token(cms)?;
    Some((|| {
        let sig = signature_value(cms).ok_or("no signature value")?;
        // wrap as a TimeStampResp with status 0 so the existing parser can read it
        let resp = der_tlv(
            0x30,
            &[der_tlv(0x30, &[0x02, 0x01, 0x00]), token.clone()].concat(),
        );
        let r = crate::tsa::parse_reply(&resp).map_err(|e| e.to_string())?;
        if r.digest != legatorocr_pdf::crypt::sha256(&sig) {
            return Err("the time stamp is for a different signature value".to_string());
        }
        if let Some(ca) = tsa_ca {
            let t = tmp.join(format!("tok{i}.der"));
            let v = tmp.join(format!("val{i}.bin"));
            std::fs::write(&t, &token).map_err(|e| e.to_string())?;
            std::fs::write(&v, &sig).map_err(|e| e.to_string())?;
            let o = openssl()
                .args(["ts", "-verify", "-token_in", "-in"])
                .arg(&t)
                .arg("-data")
                .arg(&v)
                .arg("-CAfile")
                .arg(ca)
                .output()
                .map_err(|e| format!("cannot run openssl: {e}"))?;
            if !o.status.success() {
                return Err(format!(
                    "the authority's signature was not accepted: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                ));
            }
        }
        Ok(r.gen_time)
    })())
}

/// Check every signature in `file`. Without `ca` the certificate chain is not checked
/// (`-noverify`): only that the data matches the signature.
pub fn verify_pdf(file: &Path, ca: Option<&Path>, tsa_ca: Option<&Path>) -> Result<Vec<SigReport>> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let sigs = sign::signatures(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let tmp = TempDir::new()?;
    let mut out = Vec::new();
    for (i, s) in sigs.iter().enumerate() {
        let result = (|| -> std::result::Result<(), String> {
            let signed = s
                .signed_data(&data)
                .ok_or("the byte range does not fit the file")?;
            // DER length tells where the CMS structure ends; the rest is zero padding
            let der = der_trim(&s.contents).ok_or("the signature value is not DER")?;
            let c = tmp.0.join(format!("c{i}.bin"));
            let d = tmp.0.join(format!("s{i}.der"));
            std::fs::write(&c, signed).map_err(|e| e.to_string())?;
            std::fs::write(&d, der).map_err(|e| e.to_string())?;
            let mut cmd = openssl();
            cmd.args(["cms", "-verify", "-binary", "-inform", "DER", "-in"])
                .arg(&d)
                .arg("-content")
                .arg(&c)
                .arg("-out")
                .arg(tmp.0.join(format!("o{i}.bin")));
            match ca {
                Some(p) => {
                    cmd.arg("-CAfile").arg(p);
                }
                None => {
                    cmd.arg("-noverify");
                }
            }
            let o = cmd
                .output()
                .map_err(|e| format!("cannot run openssl: {e}"))?;
            if o.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
            }
        })();
        let timestamp =
            der_trim(&s.contents).and_then(|d| check_embedded_timestamp(d, tsa_ca, &tmp.0, i));
        out.push(SigReport {
            index: i + 1,
            name: s.name.clone(),
            sub_filter: s.sub_filter.clone(),
            covers_whole_file: s.covers_whole_file(&data),
            result,
            timestamp,
        });
    }
    Ok(out)
}

/// Cut a BER/DER element to its declared length (drops padding after it).
fn der_trim(d: &[u8]) -> Option<&[u8]> {
    if d.len() < 2 || d[0] != 0x30 {
        return None;
    }
    let l0 = d[1] as usize;
    let (len, off) = if l0 < 128 {
        (l0, 2)
    } else {
        let n = l0 & 0x7f;
        if n == 0 || n > 4 || d.len() < 2 + n {
            return None;
        }
        (
            d[2..2 + n]
                .iter()
                .fold(0usize, |a, &b| (a << 8) | b as usize),
            2 + n,
        )
    };
    d.get(..off + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These tests sign with `openssl cms -cades`: skipped where no OpenSSL 3 is installed.
    fn have_openssl() -> bool {
        openssl_program().is_some()
    }

    #[test]
    fn sign_verify_and_detect_tampering() {
        if !have_openssl() {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let d = &tmp.0;
        let ok = |c: &mut Command| c.output().unwrap().status.success();
        assert!(ok(openssl()
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "2",
                "-subj",
                "/CN=Test Signer",
                "-keyout"
            ])
            .arg(d.join("k.pem"))
            .arg("-out")
            .arg(d.join("c.pem"))));
        assert!(ok(openssl()
            .args(["pkcs12", "-export", "-passout", "pass:pw", "-inkey"])
            .arg(d.join("k.pem"))
            .arg("-in")
            .arg(d.join("c.pem"))
            .arg("-out")
            .arg(d.join("t.p12"))));
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
        std::fs::write(d.join("in.pdf"), w.finish()).unwrap();
        std::env::set_var("LGR_TEST_P12", "pw");
        sign_pdf(
            &d.join("in.pdf"),
            &d.join("out.pdf"),
            &d.join("t.p12"),
            "LGR_TEST_P12",
            Some("Test"),
            Some("Approved"),
            None,
        )
        .unwrap();
        let r = verify_pdf(&d.join("out.pdf"), Some(&d.join("c.pem")), None).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].result.is_ok(), "{:?}", r[0].result);
        assert!(r[0].covers_whole_file);
        // flip one byte inside the signed range
        let mut b = std::fs::read(d.join("out.pdf")).unwrap();
        b[20] ^= 0x01;
        std::fs::write(d.join("bad.pdf"), &b).unwrap();
        let r = verify_pdf(&d.join("bad.pdf"), None, None).unwrap();
        assert!(r[0].result.is_err());
        // wrong password
        std::env::set_var("LGR_TEST_P12", "nope");
        assert!(sign_pdf(
            &d.join("in.pdf"),
            &d.join("x.pdf"),
            &d.join("t.p12"),
            "LGR_TEST_P12",
            None,
            None,
            None
        )
        .is_err());
    }

    /// PAdES-B-T: a time stamp from a local OpenSSL authority is put inside a CMS signature;
    /// OpenSSL must still verify the signature, show the attribute, and accept the token
    /// for exactly the signature value.
    #[test]
    fn a_time_stamp_token_is_added_as_an_unsigned_attribute() {
        if !have_openssl() {
            return;
        }
        let d = std::env::temp_dir().join(format!("lgr_bt_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let run = |args: &[&str]| {
            let o = openssl()
                .args(args)
                .current_dir(&d)
                .output()
                .unwrap();
            assert!(
                o.status.success(),
                "openssl {args:?}: {}",
                String::from_utf8_lossy(&o.stderr)
            );
            o
        };
        std::fs::write(d.join("tsa.cnf"), "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ext\n[dn]\nCN=Test TSA\n[ext]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n[tsa]\ndefault_tsa=t\n[t]\ndir=.\nserial=./serial\nsigner_cert=./tsa.pem\nsigner_key=./tsakey.pem\ncerts=./tsa.pem\ndefault_policy=1.2.3.4.5\ndigests=sha256\nsigner_digest=sha256\naccuracy=secs:1\nordering=yes\ntsa_name=yes\n").unwrap();
        std::fs::write(d.join("serial"), "01\n").unwrap();
        run(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-keyout",
            "tsakey.pem",
            "-out",
            "tsa.pem",
            "-days",
            "2",
            "-config",
            "tsa.cnf",
        ]);
        run(&[
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "2",
            "-subj",
            "/CN=Signer",
            "-keyout",
            "k.pem",
            "-out",
            "c.pem",
        ]);
        std::fs::write(d.join("data.bin"), b"signed bytes").unwrap();
        run(&[
            "cms", "-sign", "-binary", "-md", "sha256", "-cades", "-outform", "DER", "-in",
            "data.bin", "-signer", "c.pem", "-inkey", "k.pem", "-out", "sig.der",
        ]);
        let cms = std::fs::read(d.join("sig.der")).unwrap();
        let sigval = signature_value(&cms).expect("signature value");
        assert_eq!(sigval.len(), 256, "RSA-2048 signature");
        let mut dg = [0u8; 32];
        dg.copy_from_slice(&legatorocr_pdf::crypt::sha256(&sigval));
        std::fs::write(d.join("req.tsq"), crate::tsa::build_request(&dg, 77)).unwrap();
        run(&[
            "ts",
            "-reply",
            "-queryfile",
            "req.tsq",
            "-config",
            "tsa.cnf",
            "-out",
            "resp.tsr",
        ]);
        let resp = std::fs::read(d.join("resp.tsr")).unwrap();
        let stamped = add_timestamp(&cms, &resp).unwrap();
        assert!(stamped.len() > cms.len() + 500);
        std::fs::write(d.join("stamped.der"), &stamped).unwrap();
        // the signature is still good, and the attribute is there
        run(&[
            "cms",
            "-verify",
            "-binary",
            "-inform",
            "DER",
            "-in",
            "stamped.der",
            "-content",
            "data.bin",
            "-noverify",
            "-out",
            "out.bin",
        ]);
        let dump = String::from_utf8_lossy(
            &run(&["asn1parse", "-inform", "DER", "-in", "stamped.der"]).stdout,
        )
        .into_owned();
        assert!(dump.contains("id-smime-aa-timeStampToken"), "{dump}");
        // the token stamps exactly the signature value
        std::fs::write(d.join("sigval.bin"), &sigval).unwrap();
        run(&[
            "ts",
            "-verify",
            "-in",
            "resp.tsr",
            "-data",
            "sigval.bin",
            "-CAfile",
            "tsa.pem",
        ]);
        // and the signature value read back from the stamped CMS is unchanged
        assert_eq!(signature_value(&stamped).unwrap(), sigval);
        assert!(
            add_timestamp(&stamped, &resp).is_err(),
            "no second time stamp"
        );
        // the checker reads it back: time, right signature, and the authority when its certificate is given
        let tmp = d.clone();
        let t = check_embedded_timestamp(&stamped, None, &tmp, 0)
            .expect("has a stamp")
            .expect("good stamp");
        assert!(t.ends_with('Z'), "{t}");
        check_embedded_timestamp(&stamped, Some(&d.join("tsa.pem")), &tmp, 1)
            .unwrap()
            .unwrap();
        assert!(
            check_embedded_timestamp(&stamped, Some(&d.join("c.pem")), &tmp, 2)
                .unwrap()
                .is_err(),
            "wrong authority"
        );
        assert!(
            check_embedded_timestamp(&cms, None, &tmp, 3).is_none(),
            "unstamped signature"
        );
        let _ = std::fs::remove_dir_all(&d);
    }
}
