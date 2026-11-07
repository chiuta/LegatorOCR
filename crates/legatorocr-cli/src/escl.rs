//! Scanning from a network scanner over eSCL (AirScan / Mopria), plain HTTP only.
//!
//! No dependency beyond the standard library: a small HTTP/1.1 client over `TcpStream`, and the
//! XML of eSCL written and read with string handling (the documents are tiny and fixed).
//! Not supported here: HTTPS scanners (no TLS in this program), device discovery (mDNS), and
//! document formats other than JPEG. Tested against a stand-in server in this file's tests,
//! **not against a real scanner**.

use anyhow::{anyhow, bail, Context, Result};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Largest reply accepted from the device (one scanned page as JPEG).
const MAX_BODY: usize = 256 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub dpi: u32,
    /// "color", "gray" or "bw".
    pub mode: String,
    /// "platen" or "feeder".
    pub source: String,
    /// Stop after this many pages (0 = until the device has no more).
    pub max_pages: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            dpi: 300,
            mode: "color".into(),
            source: "platen".into(),
            max_pages: 0,
        }
    }
}

/// `host`, `host:port`, `http://host[:port]` or any of these followed by `/resource-root`
/// (default `/eSCL`).
#[derive(Debug, PartialEq)]
pub struct Target {
    pub host: String,
    pub port: u16,
    pub root: String,
}

pub fn parse_target(text: &str) -> Result<Target> {
    let t = text.trim();
    if t.starts_with("https://") {
        bail!("{text}: HTTPS scanners are not supported (this program has no TLS); use the scanner's http port");
    }
    let t = t.strip_prefix("http://").unwrap_or(t);
    let (hostport, root) = match t.find('/') {
        Some(i) => (&t[..i], t[i..].trim_end_matches('/').to_string()),
        None => (t, String::new()),
    };
    if hostport.is_empty() {
        bail!("empty scanner address");
    }
    let (host, port) = match hostport.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') || h.starts_with('[') => (
            h.trim_matches(['[', ']']).to_string(),
            p.parse::<u16>()
                .with_context(|| format!("{text}: bad port"))?,
        ),
        _ => (hostport.trim_matches(['[', ']']).to_string(), 80),
    };
    let root = if root.is_empty() {
        "/eSCL".to_string()
    } else {
        root
    };
    Ok(Target { host, port, root })
}

struct Reply {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

fn find_sub(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

fn decode_chunked(mut b: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    loop {
        let eol = find_sub(b, b"\r\n").ok_or_else(|| anyhow!("truncated chunked reply"))?;
        let size_text = std::str::from_utf8(&b[..eol])?
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        let size = usize::from_str_radix(size_text, 16).context("bad chunk size")?;
        b = &b[eol + 2..];
        if size == 0 {
            return Ok(out);
        }
        if size > MAX_BODY || out.len() + size > MAX_BODY || b.len() < size + 2 {
            bail!("chunked reply too large or truncated");
        }
        out.extend_from_slice(&b[..size]);
        b = &b[size + 2..];
    }
}

fn request(
    t: &Target,
    method: &str,
    path: &str,
    body: Option<(&str, &[u8])>,
    timeout: Duration,
) -> Result<Reply> {
    let addr = (t.host.as_str(), t.port)
        .to_socket_addrs()
        .with_context(|| format!("{}: cannot resolve", t.host))?
        .next()
        .ok_or_else(|| anyhow!("{}: no address", t.host))?;
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(10))
        .with_context(|| format!("{}:{}: cannot connect", t.host, t.port))?;
    s.set_read_timeout(Some(timeout))?;
    s.set_write_timeout(Some(Duration::from_secs(30)))?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\nUser-Agent: LegatorOCR\r\n", t.host, t.port);
    if let Some((ctype, b)) = body {
        head.push_str(&format!(
            "Content-Type: {ctype}\r\nContent-Length: {}\r\n",
            b.len()
        ));
    }
    head.push_str("\r\n");
    s.write_all(head.as_bytes())?;
    if let Some((_, b)) = body {
        s.write_all(b)?;
    }
    let mut raw = Vec::new();
    let mut limited = (&mut s).take(MAX_BODY as u64 + 65536);
    limited
        .read_to_end(&mut raw)
        .with_context(|| format!("{method} {path}: reading the reply failed"))?;
    let split = find_sub(&raw, b"\r\n\r\n")
        .ok_or_else(|| anyhow!("{method} {path}: the reply has no header"))?;
    let header = String::from_utf8_lossy(&raw[..split]).into_owned();
    let mut lines = header.lines();
    let status_line = lines.next().unwrap_or("");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|x| x.parse().ok())
        .ok_or_else(|| anyhow!("bad status line: {status_line}"))?;
    let mut location = None;
    let mut chunked = false;
    let mut length: Option<usize> = None;
    for l in lines {
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "location" => location = Some(v.trim().to_string()),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            "content-length" => length = v.trim().parse().ok(),
            _ => {}
        }
    }
    let rest = &raw[split + 4..];
    let body = if chunked {
        decode_chunked(rest)?
    } else if let Some(n) = length {
        if rest.len() < n {
            bail!(
                "{method} {path}: the reply was cut short ({} of {n} bytes)",
                rest.len()
            );
        }
        rest[..n].to_vec()
    } else {
        rest.to_vec()
    };
    if body.len() > MAX_BODY {
        bail!(
            "{method} {path}: the reply is larger than {} MB",
            MAX_BODY >> 20
        );
    }
    Ok(Reply {
        status,
        location,
        body,
    })
}

/// Text between `<…:Name>` and `</…:Name>` (any namespace prefix), first occurrence.
fn xml_value(xml: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = xml[from..].find(name) {
        let at = from + i;
        let before = &xml[..at];
        let open = before.rfind('<')?;
        // The name must start right after `<` or after a namespace prefix `x:`.
        let prefix_ok = !before[open + 1..].contains('>')
            && !before[open + 1..].starts_with('/')
            && (before.ends_with('<') || before.ends_with(':'));
        let after = &xml[at + name.len()..];
        if prefix_ok && after.starts_with('>') {
            let end = after.find('<')?;
            return Some(after[1..end].trim().to_string());
        }
        from = at + name.len();
    }
    None
}

fn color_mode(mode: &str) -> Result<&'static str> {
    Ok(match mode {
        "color" => "RGB24",
        "gray" => "Grayscale8",
        "bw" => "BlackAndWhite1",
        o => bail!("--scan-mode {o}: use color, gray or bw"),
    })
}

fn input_source(source: &str) -> Result<&'static str> {
    Ok(match source {
        "platen" | "flatbed" => "Platen",
        "feeder" | "adf" => "Feeder",
        o => bail!("--scan-source {o}: use platen or feeder"),
    })
}

/// The `ScanSettings` document for one job. Width and height are in 1/300 inch.
pub fn scan_settings(o: &Options, width: u32, height: u32) -> Result<String> {
    if !(75..=1200).contains(&o.dpi) {
        bail!("--scan-dpi {}: use 75 to 1200", o.dpi);
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<scan:ScanSettings xmlns:pwg=\"http://www.pwg.org/schemas/2010/12/sm\" xmlns:scan=\"http://schemas.hp.com/imaging/escl/2011/05/03\">\n\
<pwg:Version>2.0</pwg:Version>\n\
<pwg:ScanRegions><pwg:ScanRegion><pwg:ContentRegionUnits>escl:ThreeHundredthsOfInches</pwg:ContentRegionUnits>\
<pwg:XOffset>0</pwg:XOffset><pwg:YOffset>0</pwg:YOffset><pwg:Width>{width}</pwg:Width><pwg:Height>{height}</pwg:Height></pwg:ScanRegion></pwg:ScanRegions>\n\
<pwg:InputSource>{}</pwg:InputSource>\n\
<scan:ColorMode>{}</scan:ColorMode>\n\
<scan:XResolution>{}</scan:XResolution><scan:YResolution>{}</scan:YResolution>\n\
<pwg:DocumentFormat>image/jpeg</pwg:DocumentFormat>\n\
</scan:ScanSettings>\n",
        input_source(&o.source)?,
        color_mode(&o.mode)?,
        o.dpi,
        o.dpi
    ))
}

/// Path part of a `Location` header (absolute URL or path).
fn location_path(loc: &str) -> String {
    let l = loc.trim();
    let l = l
        .strip_prefix("http://")
        .map(|r| r.find('/').map_or("/", |i| &r[i..]))
        .unwrap_or(l);
    l.trim_end_matches('/').to_string()
}

/// Scan from the device and write `scan-0001.jpg`, … into `dir`. Returns the files in order.
pub fn scan(target: &str, o: &Options, dir: &Path) -> Result<Vec<PathBuf>> {
    let t = parse_target(target)?;
    let short = Duration::from_secs(30);
    let caps = request(
        &t,
        "GET",
        &format!("{}/ScannerCapabilities", t.root),
        None,
        short,
    )?;
    if caps.status != 200 {
        bail!(
            "{target}: ScannerCapabilities answered HTTP {} (is this an eSCL scanner?)",
            caps.status
        );
    }
    let caps_xml = String::from_utf8_lossy(&caps.body).into_owned();
    let (mut w, mut h) = (2480u32, 3508u32); // A4 in 1/300 inch when the device does not say
                                             // Several input sources can list different sizes; the first MaxWidth/MaxHeight pair is used.
    if let (Some(mw), Some(mh)) = (
        xml_value(&caps_xml, "MaxWidth"),
        xml_value(&caps_xml, "MaxHeight"),
    ) {
        if let (Ok(a), Ok(b)) = (mw.parse::<u32>(), mh.parse::<u32>()) {
            if a > 0 && b > 0 {
                w = a;
                h = b;
            }
        }
    }
    let settings = scan_settings(o, w, h)?;
    let job = request(
        &t,
        "POST",
        &format!("{}/ScanJobs", t.root),
        Some(("text/xml", settings.as_bytes())),
        short,
    )?;
    if job.status != 201 {
        bail!("{target}: the scanner refused the job (HTTP {}); busy, out of paper or the source/mode is not offered", job.status);
    }
    let job_path = location_path(
        job.location
            .as_deref()
            .ok_or_else(|| anyhow!("{target}: no Location for the new job"))?,
    );
    std::fs::create_dir_all(dir)
        .with_context(|| format!("{}: cannot create the folder", dir.display()))?;
    let mut files = Vec::new();
    let started = Instant::now();
    loop {
        if o.max_pages > 0 && files.len() >= o.max_pages {
            break;
        }
        let r = request(
            &t,
            "GET",
            &format!("{job_path}/NextDocument"),
            None,
            Duration::from_secs(180),
        )?;
        match r.status {
            200 => {
                if r.body.len() < 4 || r.body[..2] != [0xFF, 0xD8] {
                    bail!("{target}: page {} is not a JPEG (the scanner ignored the requested format)", files.len() + 1);
                }
                let p = dir.join(format!("scan-{:04}.jpg", files.len() + 1));
                legatorocr_engine::write_atomic(&p, &r.body)?;
                files.push(p);
            }
            // 503: warming up or busy; 404/410: no more pages.
            503 if started.elapsed() < Duration::from_secs(90) => {
                std::thread::sleep(Duration::from_secs(2))
            }
            404 | 410 => break,
            c => bail!("{target}: NextDocument answered HTTP {c}"),
        }
        if o.source == "platen" || o.source == "flatbed" {
            break; // a flatbed gives one page per job
        }
    }
    if files.is_empty() {
        bail!("{target}: the scanner gave no page (feeder empty?)");
    }
    // Best-effort cleanup; many devices end the job by themselves.
    let _ = request(&t, "DELETE", &job_path, None, Duration::from_secs(5));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    fn jpeg_bytes(v: u8) -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([v, v, v]));
        let mut out = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut out, image::ImageFormat::Jpeg)
            .unwrap();
        out.into_inner()
    }

    /// Stand-in eSCL device: `pages` documents, then 404. Returns the port; logs requests.
    fn serve(pages: usize, chunked: bool) -> (u16, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log2 = log.clone();
        std::thread::spawn(move || {
            let mut served = 0usize;
            for s in l.incoming() {
                let Ok(mut s) = s else { break };
                let mut buf = vec![0u8; 65536];
                let mut n = 0;
                // read the header and any body
                loop {
                    let k = s.read(&mut buf[n..]).unwrap_or(0);
                    n += k;
                    if k == 0
                        || find_sub(&buf[..n], b"\r\n\r\n").is_some_and(|i| {
                            let head = String::from_utf8_lossy(&buf[..i]).to_ascii_lowercase();
                            let want = head
                                .split("content-length:")
                                .nth(1)
                                .and_then(|x| x.lines().next())
                                .and_then(|x| x.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            n >= i + 4 + want
                        })
                    {
                        break;
                    }
                }
                let req = String::from_utf8_lossy(&buf[..n]).into_owned();
                let first = req.lines().next().unwrap_or("").to_string();
                log2.lock().unwrap().push(req.clone());
                let (status, extra, body): (&str, String, Vec<u8>) = if first
                    .starts_with("GET /eSCL/ScannerCapabilities")
                {
                    ("200 OK", String::new(), b"<scan:ScannerCapabilities><scan:Platen><scan:MaxWidth>2550</scan:MaxWidth><scan:MaxHeight>3507</scan:MaxHeight></scan:Platen></scan:ScannerCapabilities>".to_vec())
                } else if first.starts_with("POST /eSCL/ScanJobs") {
                    (
                        "201 Created",
                        "Location: http://127.0.0.1/eSCL/ScanJobs/7\r\n".into(),
                        Vec::new(),
                    )
                } else if first.starts_with("GET /eSCL/ScanJobs/7/NextDocument") {
                    if served < pages {
                        served += 1;
                        (
                            "200 OK",
                            "Content-Type: image/jpeg\r\n".into(),
                            jpeg_bytes(served as u8 * 40),
                        )
                    } else {
                        ("404 Not Found", String::new(), Vec::new())
                    }
                } else {
                    ("200 OK", String::new(), Vec::new())
                };
                let mut out = format!("HTTP/1.1 {status}\r\n{extra}");
                if chunked && !body.is_empty() {
                    out.push_str("Transfer-Encoding: chunked\r\n\r\n");
                    let mut o = out.into_bytes();
                    for part in body.chunks(100) {
                        o.extend_from_slice(format!("{:x}\r\n", part.len()).as_bytes());
                        o.extend_from_slice(part);
                        o.extend_from_slice(b"\r\n");
                    }
                    o.extend_from_slice(b"0\r\n\r\n");
                    let _ = s.write_all(&o);
                } else {
                    out.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
                    let mut o = out.into_bytes();
                    o.extend_from_slice(&body);
                    let _ = s.write_all(&o);
                }
            }
        });
        (port, log)
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("legatorocr-escl-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn targets_parse() {
        assert_eq!(
            parse_target("192.168.1.9").unwrap(),
            Target {
                host: "192.168.1.9".into(),
                port: 80,
                root: "/eSCL".into()
            }
        );
        assert_eq!(
            parse_target("http://printer.local:8080/scanner/eSCL/").unwrap(),
            Target {
                host: "printer.local".into(),
                port: 8080,
                root: "/scanner/eSCL".into()
            }
        );
        assert!(parse_target("https://x").is_err());
        assert!(parse_target("").is_err());
        assert!(parse_target("host:notaport").is_err());
    }

    #[test]
    fn settings_validate_and_carry_values() {
        let o = Options {
            dpi: 600,
            mode: "gray".into(),
            source: "feeder".into(),
            max_pages: 0,
        };
        let x = scan_settings(&o, 2550, 3507).unwrap();
        assert!(
            x.contains("<scan:XResolution>600<")
                && x.contains("Grayscale8")
                && x.contains("Feeder")
                && x.contains("<pwg:Width>2550<")
        );
        assert!(scan_settings(
            &Options {
                dpi: 10,
                ..Options::default()
            },
            1,
            1
        )
        .is_err());
        assert!(scan_settings(
            &Options {
                mode: "x".into(),
                ..Options::default()
            },
            1,
            1
        )
        .is_err());
        assert_eq!(
            xml_value("<a:MaxWidth>12</a:MaxWidth>", "MaxWidth").as_deref(),
            Some("12")
        );
        assert_eq!(xml_value("<a:XMaxWidth>12</a:XMaxWidth>", "MaxWidth"), None);
    }

    #[test]
    fn flatbed_one_page_uses_device_size() {
        let (port, log) = serve(1, false);
        let d = tmp("flat");
        let files = scan(&format!("127.0.0.1:{port}"), &Options::default(), &d).unwrap();
        assert_eq!(files.len(), 1);
        assert!(std::fs::read(&files[0]).unwrap().starts_with(&[0xFF, 0xD8]));
        let posted = log
            .lock()
            .unwrap()
            .iter()
            .find(|r| r.starts_with("POST"))
            .cloned()
            .unwrap();
        assert!(
            posted.contains("<pwg:Width>2550<")
                && posted.contains("<pwg:Height>3507<")
                && posted.contains("RGB24")
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn feeder_reads_until_404_and_handles_chunked() {
        let (port, _) = serve(3, true);
        let d = tmp("adf");
        let o = Options {
            source: "feeder".into(),
            ..Options::default()
        };
        let files = scan(&format!("127.0.0.1:{port}"), &o, &d).unwrap();
        assert_eq!(files.len(), 3);
        assert!(files[2].ends_with("scan-0003.jpg"));
        for f in &files {
            image::open(f).unwrap(); // every file decodes: the chunks were put back together
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn page_limit_and_empty_feeder() {
        let (port, _) = serve(5, false);
        let d = tmp("lim");
        let o = Options {
            source: "feeder".into(),
            max_pages: 2,
            ..Options::default()
        };
        assert_eq!(scan(&format!("127.0.0.1:{port}"), &o, &d).unwrap().len(), 2);
        let (port0, _) = serve(0, false);
        let e = scan(
            &format!("127.0.0.1:{port0}"),
            &Options {
                source: "feeder".into(),
                ..Options::default()
            },
            &d,
        )
        .unwrap_err();
        assert!(e.to_string().contains("no page"), "{e}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn unreachable_device_is_an_error_not_a_hang() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        assert!(scan(
            &format!("127.0.0.1:{port}"),
            &Options::default(),
            &tmp("none")
        )
        .is_err());
    }
}
