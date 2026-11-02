//! RFC 3161 time stamps (roadmap 5.4): build a request for the SHA-256 of a file, send it to a
//! time-stamping authority with `curl`, check what came back (status, hash, nonce, time) and
//! store the reply as `FILE.tsr`. Checking the signature and certificate chain of the reply is
//! left to `openssl ts -verify`, which `verify` runs when OpenSSL is installed.
//!
//! No authority is built in and none was contacted while this was written (the network of the
//! build environment is restricted): the encoder and parser were tested against requests and
//! replies made by OpenSSL with a throw-away authority created in the test.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

const SHA256_OID: [u8; 9] = [0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01];

fn tlv(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut v = vec![tag];
    let n = body.len();
    if n < 128 {
        v.push(n as u8);
    } else if n < 256 {
        v.extend([0x81, n as u8]);
    } else {
        v.extend([0x82, (n >> 8) as u8, n as u8]);
    }
    v.extend_from_slice(body);
    v
}

fn int(mut n: u64) -> Vec<u8> {
    let mut b = Vec::new();
    loop {
        b.insert(0, (n & 0xff) as u8);
        n >>= 8;
        if n == 0 {
            break;
        }
    }
    if b[0] & 0x80 != 0 {
        b.insert(0, 0);
    }
    tlv(0x02, &b)
}

/// DER `TimeStampReq` for a SHA-256 digest, asking for the authority's certificate.
pub fn build_request(digest: &[u8; 32], nonce: u64) -> Vec<u8> {
    let alg = tlv(0x30, &[tlv(0x06, &SHA256_OID), vec![0x05, 0x00]].concat());
    let imprint = tlv(0x30, &[alg, tlv(0x04, digest)].concat());
    tlv(0x30, &[int(1), imprint, int(nonce), tlv(0x01, &[0xff])].concat())
}

/// Read one DER element: (tag, body, rest).
fn read(d: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *d.first()?;
    let l0 = *d.get(1)? as usize;
    let (len, off) = if l0 < 128 {
        (l0, 2)
    } else {
        let n = l0 & 0x7f;
        if n == 0 || n > 3 {
            return None;
        }
        let mut l = 0usize;
        for i in 0..n {
            l = (l << 8) | *d.get(2 + i)? as usize;
        }
        (l, 2 + n)
    };
    let body = d.get(off..off.checked_add(len)?)?;
    Some((tag, body, &d[off + len..]))
}

#[derive(Debug, PartialEq)]
pub struct Reply {
    /// PKIStatus: 0 granted, 1 granted with modifications.
    pub status: u64,
    pub digest: Vec<u8>,
    pub nonce: Option<u64>,
    /// GeneralizedTime as written, e.g. `20261102163012Z`.
    pub gen_time: String,
}

fn uint(b: &[u8]) -> u64 {
    b.iter().fold(0u64, |a, &x| (a << 8) | x as u64)
}

/// Pull status, message imprint, nonce and time out of a DER `TimeStampResp`.
pub fn parse_reply(der: &[u8]) -> Result<Reply> {
    let bad = || anyhow::anyhow!("not a valid time-stamp reply");
    let (_, resp, _) = read(der).ok_or_else(bad)?;
    let (_, status_info, rest) = read(resp).ok_or_else(bad)?;
    let (_, st, _) = read(status_info).ok_or_else(bad)?;
    let status = uint(st);
    if status > 1 {
        bail!("the authority refused the request (PKIStatus {status})");
    }
    // timeStampToken: ContentInfo { oid, [0] SignedData }
    let (_, ci, _) = read(rest).ok_or_else(bad)?;
    let (_, _oid, ci) = read(ci).ok_or_else(bad)?;
    let (_, expl, _) = read(ci).ok_or_else(bad)?;
    let (_, sd, _) = read(expl).ok_or_else(bad)?;
    // SignedData: version, digestAlgorithms, encapContentInfo, ...
    let (_, _v, sd) = read(sd).ok_or_else(bad)?;
    let (_, _algs, sd) = read(sd).ok_or_else(bad)?;
    let (_, encap, _) = read(sd).ok_or_else(bad)?;
    let (_, _ct, encap) = read(encap).ok_or_else(bad)?;
    let (_, e0, _) = read(encap).ok_or_else(bad)?;
    let (_, octets, _) = read(e0).ok_or_else(bad)?;
    // TSTInfo: version, policy, messageImprint, serial, genTime, [accuracy], [ordering], [nonce] ...
    let (_, tst, _) = read(octets).ok_or_else(bad)?;
    let (_, _ver, t) = read(tst).ok_or_else(bad)?;
    let (_, _pol, t) = read(t).ok_or_else(bad)?;
    let (_, imprint, t) = read(t).ok_or_else(bad)?;
    let (_, _alg, imp) = read(imprint).ok_or_else(bad)?;
    let (_, digest, _) = read(imp).ok_or_else(bad)?;
    let (_, _serial, t) = read(t).ok_or_else(bad)?;
    let (_, gen, mut t) = read(t).ok_or_else(bad)?;
    let mut nonce = None;
    while let Some((tag, body, rest)) = read(t) {
        if tag == 0x02 {
            nonce = Some(uint(body));
        }
        t = rest;
    }
    Ok(Reply { status, digest: digest.to_vec(), nonce, gen_time: String::from_utf8_lossy(gen).into_owned() })
}

fn digest_of(data: &[u8]) -> [u8; 32] {
    let v = legatorocr_pdf::crypt::sha256(data);
    let mut d = [0u8; 32];
    d.copy_from_slice(&v);
    d
}

fn nonce() -> u64 {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    u64::from_be_bytes(b) >> 1
}

pub fn tsr_path(file: &Path) -> PathBuf {
    let mut s = file.as_os_str().to_os_string();
    s.push(".tsr");
    PathBuf::from(s)
}

/// Time-stamp `file` through `url` (HTTP POST with curl); writes `FILE.tsr` and returns the time.
pub fn stamp(file: &Path, url: &str) -> Result<Reply> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        bail!("--tsa-url must be an http:// or https:// address");
    }
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let digest = digest_of(&data);
    let n = nonce();
    let req = build_request(&digest, n);
    let tmp = std::env::temp_dir().join(format!("lgr_tsq_{}.tsq", std::process::id()));
    std::fs::write(&tmp, &req)?;
    let out = Command::new("curl")
        .args(["-sS", "--fail", "--max-time", "60", "-H", "Content-Type: application/timestamp-query", "--data-binary"])
        .arg(format!("@{}", tmp.display()))
        .arg(url)
        .output()
        .context("cannot run curl (needed to reach the time-stamp authority)");
    let _ = std::fs::remove_file(&tmp);
    let out = out?;
    if !out.status.success() {
        bail!("the request to {url} failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    let reply = parse_reply(&out.stdout)?;
    if reply.digest != digest {
        bail!("the reply is for a different hash than the file's");
    }
    if reply.nonce != Some(n) {
        bail!("the reply does not carry our nonce (possible replay)");
    }
    std::fs::write(tsr_path(file), &out.stdout)?;
    Ok(reply)
}

/// Check `FILE.tsr` against `file`: hash and structure always; signature and chain through
/// `openssl ts -verify` when `ca_file` is given (needs OpenSSL on the PATH).
pub fn verify(file: &Path, ca_file: Option<&Path>) -> Result<Reply> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let tsr = tsr_path(file);
    let der = std::fs::read(&tsr).with_context(|| format!("reading {}", tsr.display()))?;
    let r = parse_reply(&der)?;
    if r.digest != digest_of(&data) {
        bail!("{} does not match the time stamp: the file changed after it was stamped", file.display());
    }
    if let Some(ca) = ca_file {
        let o = Command::new("openssl")
            .args(["ts", "-verify", "-in"])
            .arg(&tsr)
            .arg("-data")
            .arg(file)
            .arg("-CAfile")
            .arg(ca)
            .output()
            .context("cannot run openssl")?;
        if !o.status.success() {
            bail!("openssl could not verify the signature: {}", String::from_utf8_lossy(&o.stderr).trim());
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_has_the_der_shape_openssl_expects() {
        let r = build_request(&[7u8; 32], 0x1234);
        // SEQUENCE { INTEGER 1, SEQUENCE { SEQUENCE { OID sha256, NULL }, OCTET STRING }, INTEGER, BOOLEAN }
        assert_eq!(r[0], 0x30);
        assert_eq!(&r[2..5], &[0x02, 0x01, 0x01]);
        let n = r.len();
        assert_eq!(&r[n - 3..], &[0x01, 0x01, 0xff]);
        assert!(r.windows(9).any(|w| w == SHA256_OID));
        assert_eq!(int(0x80), vec![0x02, 0x02, 0x00, 0x80]);
    }

    /// Made with the real OpenSSL: a throw-away authority signs a request built by us.
    #[test]
    fn round_trip_with_openssl_authority() {
        if Command::new("openssl").arg("version").output().is_err() {
            eprintln!("openssl not installed: skipped");
            return;
        }
        let dir = std::env::temp_dir().join(format!("lgr_tsa_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let sh = |args: &[&str]| {
            let o = Command::new("openssl").args(args).current_dir(&dir).output().unwrap();
            assert!(o.status.success(), "openssl {args:?}: {}", String::from_utf8_lossy(&o.stderr));
        };
        std::fs::write(
            dir.join("tsa.cnf"),
            "[req]\ndistinguished_name=dn\nprompt=no\nx509_extensions=ext\n[dn]\nCN=Test TSA\n[ext]\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,timeStamping\n\
             [tsa]\ndefault_tsa=tsa_cfg\n[tsa_cfg]\ndir=.\nserial=./serial\nsigner_cert=./cert.pem\nsigner_key=./key.pem\ncerts=./cert.pem\ndefault_policy=1.2.3.4.5\ndigests=sha256\nsigner_digest=sha256\naccuracy=secs:1\nordering=yes\ntsa_name=yes\n",
        )
        .unwrap();
        std::fs::write(dir.join("serial"), "01\n").unwrap();
        sh(&["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "key.pem", "-out", "cert.pem", "-days", "2", "-config", "tsa.cnf"]);
        let doc = dir.join("doc.txt");
        std::fs::write(&doc, b"registry root").unwrap();
        let digest = digest_of(b"registry root");
        std::fs::write(dir.join("req.tsq"), build_request(&digest, 0xABCDEF)).unwrap();
        sh(&["ts", "-reply", "-queryfile", "req.tsq", "-config", "tsa.cnf", "-out", "doc.txt.tsr"]);
        let r = parse_reply(&std::fs::read(dir.join("doc.txt.tsr")).unwrap()).unwrap();
        assert_eq!(r.digest, digest.to_vec());
        assert_eq!(r.nonce, Some(0xABCDEF));
        assert!(r.gen_time.ends_with('Z') && r.gen_time.len() >= 14, "{}", r.gen_time);
        // verify: structure + hash, then OpenSSL checks the signature against the TSA certificate
        verify(&doc, Some(&dir.join("cert.pem"))).unwrap();
        // a changed file is caught
        std::fs::write(&doc, b"registry root changed").unwrap();
        assert!(verify(&doc, None).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn garbage_is_not_a_reply() {
        assert!(parse_reply(b"").is_err());
        assert!(parse_reply(&[0x30, 0x03, 0x30, 0x01, 0x00]).is_err());
    }
}
