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
        let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
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

fn openssl() -> Command {
    Command::new("openssl")
}

/// Sign `input` into `output`. The PKCS#12 password is read from the environment variable
/// `pass_env` (never from the command line).
pub fn sign_pdf(input: &Path, output: &Path, p12: &Path, pass_env: &str, name: Option<&str>, reason: Option<&str>) -> Result<()> {
    let data = std::fs::read(input).with_context(|| format!("reading {}", input.display()))?;
    if std::env::var_os(pass_env).is_none() {
        bail!("set the PKCS#12 password in the environment variable {pass_env} (use an empty value for none)");
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
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
        bail!("openssl could not open the PKCS#12 file (wrong password?): {}", String::from_utf8_lossy(&st.stderr).trim());
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
    let has_chain = ok && std::fs::read_to_string(&chain).map(|t| t.contains("BEGIN CERTIFICATE")).unwrap_or(false);
    let content = tmp.0.join("content.bin");
    std::fs::write(&content, prepared.signed_data())?;
    let der = tmp.0.join("sig.der");
    let mut cmd = openssl();
    cmd.args(["cms", "-sign", "-binary", "-md", "sha256", "-cades", "-outform", "DER", "-in"])
        .arg(&content)
        .arg("-signer")
        .arg(&pem)
        .arg("-inkey")
        .arg(&pem);
    if has_chain {
        cmd.arg("-certfile").arg(&chain);
    }
    let st = cmd.arg("-out").arg(&der).output().context("cannot run openssl")?;
    if !st.status.success() {
        bail!("openssl cms -sign failed: {}", String::from_utf8_lossy(&st.stderr).trim());
    }
    let signed = prepared.embed(&std::fs::read(&der)?).map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(output, signed)?;
    Ok(())
}

pub struct SigReport {
    pub index: usize,
    pub name: Option<String>,
    pub sub_filter: String,
    pub covers_whole_file: bool,
    /// `Ok(())` when openssl accepted the signature.
    pub result: std::result::Result<(), String>,
}

/// Check every signature in `file`. Without `ca` the certificate chain is not checked
/// (`-noverify`): only that the data matches the signature.
pub fn verify_pdf(file: &Path, ca: Option<&Path>) -> Result<Vec<SigReport>> {
    let data = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let sigs = sign::signatures(&data).map_err(|e| anyhow::anyhow!("{e}"))?;
    let tmp = TempDir::new()?;
    let mut out = Vec::new();
    for (i, s) in sigs.iter().enumerate() {
        let result = (|| -> std::result::Result<(), String> {
            let signed = s.signed_data(&data).ok_or("the byte range does not fit the file")?;
            // DER length tells where the CMS structure ends; the rest is zero padding
            let der = der_trim(&s.contents).ok_or("the signature value is not DER")?;
            let c = tmp.0.join(format!("c{i}.bin"));
            let d = tmp.0.join(format!("s{i}.der"));
            std::fs::write(&c, signed).map_err(|e| e.to_string())?;
            std::fs::write(&d, der).map_err(|e| e.to_string())?;
            let mut cmd = openssl();
            cmd.args(["cms", "-verify", "-binary", "-inform", "DER", "-in"]).arg(&d).arg("-content").arg(&c).arg("-out").arg(tmp.0.join(format!("o{i}.bin")));
            match ca {
                Some(p) => {
                    cmd.arg("-CAfile").arg(p);
                }
                None => {
                    cmd.arg("-noverify");
                }
            }
            let o = cmd.output().map_err(|e| format!("cannot run openssl: {e}"))?;
            if o.status.success() {
                Ok(())
            } else {
                Err(String::from_utf8_lossy(&o.stderr).trim().to_string())
            }
        })();
        out.push(SigReport { index: i + 1, name: s.name.clone(), sub_filter: s.sub_filter.clone(), covers_whole_file: s.covers_whole_file(&data), result });
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
        (d[2..2 + n].iter().fold(0usize, |a, &b| (a << 8) | b as usize), 2 + n)
    };
    d.get(..off + len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn have_openssl() -> bool {
        Command::new("openssl").arg("version").output().map(|o| o.status.success()).unwrap_or(false)
    }

    #[test]
    fn sign_verify_and_detect_tampering() {
        if !have_openssl() {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let d = &tmp.0;
        let ok = |c: &mut Command| c.output().unwrap().status.success();
        assert!(ok(openssl().args(["req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2", "-subj", "/CN=Test Signer", "-keyout"]).arg(d.join("k.pem")).arg("-out").arg(d.join("c.pem"))));
        assert!(ok(openssl().args(["pkcs12", "-export", "-passout", "pass:pw", "-inkey"]).arg(d.join("k.pem")).arg("-in").arg(d.join("c.pem")).arg("-out").arg(d.join("t.p12"))));
        let mut w = legatorocr_pdf::PdfWriter::new(legatorocr_pdf::Compress::Flate);
        w.add_page(
            &legatorocr_core::Page { image: legatorocr_core::PageImage::Bw(legatorocr_core::BitImage::new(32, 32)), dpi: 72, skew_deg: 0.0, blank: false, split_index: None, content_box: None, layers: None },
            &[],
        );
        std::fs::write(d.join("in.pdf"), w.finish()).unwrap();
        std::env::set_var("LGR_TEST_P12", "pw");
        sign_pdf(&d.join("in.pdf"), &d.join("out.pdf"), &d.join("t.p12"), "LGR_TEST_P12", Some("Test"), Some("Approved")).unwrap();
        let r = verify_pdf(&d.join("out.pdf"), Some(&d.join("c.pem"))).unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].result.is_ok(), "{:?}", r[0].result);
        assert!(r[0].covers_whole_file);
        // flip one byte inside the signed range
        let mut b = std::fs::read(d.join("out.pdf")).unwrap();
        b[20] ^= 0x01;
        std::fs::write(d.join("bad.pdf"), &b).unwrap();
        let r = verify_pdf(&d.join("bad.pdf"), None).unwrap();
        assert!(r[0].result.is_err());
        // wrong password
        std::env::set_var("LGR_TEST_P12", "nope");
        assert!(sign_pdf(&d.join("in.pdf"), &d.join("x.pdf"), &d.join("t.p12"), "LGR_TEST_P12", None, None).is_err());
    }
}
