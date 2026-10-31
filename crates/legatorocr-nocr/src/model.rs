//! The recogniser network and its binary format (`LNM1`).
//!
//! Architecture (written from the general CRNN idea: convolutional features → recurrent
//! sequence model → CTC): four 3×3 convolutions (BatchNorm folded in at export) with ReLU and
//! max-pooling reduce a 32-pixel-high line to a column sequence (width/4); two bidirectional GRU
//! layers and a linear layer give per-column class scores; class 0 is the CTC blank.

use std::collections::HashMap;

pub const LINE_H: usize = 32;
const POOLS: [(usize, usize); 4] = [(2, 2), (2, 2), (2, 1), (2, 1)];

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("not an LNM1 model file")]
    Magic,
    #[error("model file is truncated or inconsistent: {0}")]
    Corrupt(&'static str),
    #[error("model is missing tensor {0}")]
    Missing(String),
}

#[derive(Clone)]
struct Tensor {
    dims: Vec<usize>,
    data: Vec<f32>,
}

#[derive(Clone)]
struct Conv {
    cin: usize,
    cout: usize,
    /// `[ky*3+kx][cin][cout]`: output channel innermost so the inner loop vectorises.
    w: Vec<f32>,
    b: Vec<f32>,
}

#[derive(Clone)]
struct GruDir {
    /// `[in][3*hid]` and `[hid][3*hid]`, gate order r, z, n (as exported).
    wih: Vec<f32>,
    whh: Vec<f32>,
    bih: Vec<f32>,
    bhh: Vec<f32>,
}

#[derive(Clone)]
struct Bigru {
    nin: usize,
    f: GruDir,
    r: GruDir,
}

#[derive(Clone)]
pub struct Model {
    pub charset: Vec<char>,
    hid: usize,
    convs: Vec<Conv>,
    gru: [Bigru; 2],
    fc_w: Vec<f32>, // [2*hid][nc]
    fc_b: Vec<f32>,
    pub nc: usize,
}

struct Rd<'a>(&'a [u8], usize);
impl<'a> Rd<'a> {
    fn u32(&mut self) -> Result<u32, ModelError> {
        let s = self.0.get(self.1..self.1 + 4).ok_or(ModelError::Corrupt("eof"))?;
        self.1 += 4;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn bytes(&mut self, n: usize) -> Result<&'a [u8], ModelError> {
        let e = self.1.checked_add(n).ok_or(ModelError::Corrupt("size"))?;
        let s = self.0.get(self.1..e).ok_or(ModelError::Corrupt("eof"))?;
        self.1 = e;
        Ok(s)
    }
}

fn transpose(src: &[f32], rows: usize, cols: usize) -> Vec<f32> {
    let mut o = vec![0.0; src.len()];
    for r in 0..rows {
        for c in 0..cols {
            o[c * rows + r] = src[r * cols + c];
        }
    }
    o
}

impl Model {
    pub fn from_bytes(b: &[u8]) -> Result<Model, ModelError> {
        if b.get(..4) != Some(b"LNM1") {
            return Err(ModelError::Magic);
        }
        let mut r = Rd(b, 4);
        let h = r.u32()? as usize;
        let hid = r.u32()? as usize;
        let nconv = r.u32()? as usize;
        let cslen = r.u32()? as usize;
        if h != LINE_H || nconv != 4 || hid == 0 || hid > 1024 || cslen > 4096 {
            return Err(ModelError::Corrupt("header"));
        }
        let charset: Vec<char> = std::str::from_utf8(r.bytes(cslen)?)
            .map_err(|_| ModelError::Corrupt("charset"))?
            .chars()
            .collect();
        let nt = r.u32()? as usize;
        let mut t: HashMap<String, Tensor> = HashMap::new();
        for _ in 0..nt {
            let nl = r.u32()? as usize;
            let name = String::from_utf8_lossy(r.bytes(nl)?).into_owned();
            let nd = r.u32()? as usize;
            if nd > 4 {
                return Err(ModelError::Corrupt("rank"));
            }
            let mut dims = Vec::new();
            let mut n = 1usize;
            for _ in 0..nd {
                let d = r.u32()? as usize;
                n = n.checked_mul(d).ok_or(ModelError::Corrupt("dims"))?;
                dims.push(d);
            }
            let raw = r.bytes(n.checked_mul(4).ok_or(ModelError::Corrupt("dims"))?)?;
            let data = raw
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            t.insert(name, Tensor { dims, data });
        }
        let get = |n: &str, nd: usize| -> Result<&Tensor, ModelError> {
            let x = t.get(n).ok_or_else(|| ModelError::Missing(n.into()))?;
            if x.dims.len() != nd {
                return Err(ModelError::Corrupt("rank mismatch"));
            }
            Ok(x)
        };
        let mut convs = Vec::new();
        for i in 0..4 {
            let w = get(&format!("conv{i}.w"), 4)?; // [cout][cin][3][3]
            let b = get(&format!("conv{i}.b"), 1)?;
            let (cout, cin) = (w.dims[0], w.dims[1]);
            if w.dims[2] != 3 || w.dims[3] != 3 || b.data.len() != cout {
                return Err(ModelError::Corrupt("conv shape"));
            }
            let mut wt = vec![0.0; w.data.len()];
            for o in 0..cout {
                for c in 0..cin {
                    for k in 0..9 {
                        wt[(k * cin + c) * cout + o] = w.data[(o * cin + c) * 9 + k];
                    }
                }
            }
            convs.push(Conv { cin, cout, w: wt, b: b.data.clone() });
        }
        let c_last = convs[3].cout;
        let gdir = |layer: &str, d: &str, nin: usize| -> Result<GruDir, ModelError> {
            let wih = get(&format!("{layer}.{d}.weight_ih"), 2)?;
            let whh = get(&format!("{layer}.{d}.weight_hh"), 2)?;
            let bih = get(&format!("{layer}.{d}.bias_ih"), 1)?;
            let bhh = get(&format!("{layer}.{d}.bias_hh"), 1)?;
            if wih.dims != [3 * hid, nin] || whh.dims != [3 * hid, hid] || bih.data.len() != 3 * hid || bhh.data.len() != 3 * hid {
                return Err(ModelError::Corrupt("gru shape"));
            }
            Ok(GruDir {
                wih: transpose(&wih.data, 3 * hid, nin),
                whh: transpose(&whh.data, 3 * hid, hid),
                bih: bih.data.clone(),
                bhh: bhh.data.clone(),
            })
        };
        let n1 = c_last * (LINE_H / 16);
        let gru = [
            Bigru { nin: n1, f: gdir("gru1", "f", n1)?, r: gdir("gru1", "r", n1)? },
            Bigru { nin: 2 * hid, f: gdir("gru2", "f", 2 * hid)?, r: gdir("gru2", "r", 2 * hid)? },
        ];
        let nc = charset.len() + 1;
        let fw = get("fc.w", 2)?;
        let fb = get("fc.b", 1)?;
        if fw.dims != [nc, 2 * hid] || fb.data.len() != nc {
            return Err(ModelError::Corrupt("fc shape"));
        }
        Ok(Model { charset, hid, convs, gru, fc_w: transpose(&fw.data, nc, 2 * hid), fc_b: fb.data.clone(), nc })
    }

    /// Per-column log-probabilities of a line image (`LINE_H` rows, ink = 1.0, paper = 0.0, row
    /// major). Returns `(columns, nc)`; column `t` covers pixels `4t..4t+4`.
    pub fn forward(&self, img: &[f32], w: usize) -> (usize, Vec<f32>) {
        assert_eq!(img.len(), LINE_H * w);
        let w0 = w;
        let w = w.max(4).div_ceil(4) * 4;
        // channels-last activation [h][w][c] (one channel); columns past `w0` stay paper
        let mut h = LINE_H;
        let mut wd = w;
        let mut act = vec![0.0f32; h * wd];
        for y in 0..h {
            act[y * wd..y * wd + w0].copy_from_slice(&img[y * w0..(y + 1) * w0]);
        }
        let mut ch = 1;
        for (cv, &(ph, pw)) in self.convs.iter().zip(POOLS.iter()) {
            debug_assert_eq!(cv.cin, ch);
            act = conv3x3_relu(&act, h, wd, cv);
            act = maxpool(&act, h, wd, cv.cout, ph, pw);
            h /= ph;
            wd /= pw;
            ch = cv.cout;
        }
        let t = wd;
        let feat = h * ch; // per column, ordered [h][c]
        // act is [h][w][c] → sequence [w][h*c]
        let mut seq = vec![0.0f32; t * feat];
        for y in 0..h {
            for x in 0..t {
                let src = (y * t + x) * ch;
                let dst = x * feat + y * ch;
                seq[dst..dst + ch].copy_from_slice(&act[src..src + ch]);
            }
        }
        // torch flattens as [c][h]: reorder to match training (feature index = c*h + y)
        let mut seq2 = vec![0.0f32; t * feat];
        for x in 0..t {
            for y in 0..h {
                for c in 0..ch {
                    seq2[x * feat + c * h + y] = seq[x * feat + y * ch + c];
                }
            }
        }
        let mut cur = seq2;
        for g in &self.gru {
            cur = bigru(&cur, t, g, self.hid);
        }
        // linear
        let nc = self.nc;
        let mut out = vec![0.0f32; t * nc];
        for x in 0..t {
            let row = &mut out[x * nc..(x + 1) * nc];
            row.copy_from_slice(&self.fc_b);
            for (k, &v) in cur[x * 2 * self.hid..(x + 1) * 2 * self.hid].iter().enumerate() {
                let wr = &self.fc_w[k * nc..(k + 1) * nc];
                for (o, &wv) in row.iter_mut().zip(wr) {
                    *o += v * wv;
                }
            }
            log_softmax(row);
        }
        (t, out)
    }
}

fn log_softmax(r: &mut [f32]) {
    let m = r.iter().cloned().fold(f32::MIN, f32::max);
    let s: f32 = r.iter().map(|&v| (v - m).exp()).sum();
    let l = m + s.ln();
    for v in r {
        *v -= l;
    }
}

/// 3×3 convolution, padding 1, ReLU; activations `[h][w][c]`.
fn conv3x3_relu(x: &[f32], h: usize, w: usize, cv: &Conv) -> Vec<f32> {
    let (cin, cout) = (cv.cin, cv.cout);
    let mut out = vec![0.0f32; h * w * cout];
    for y in 0..h {
        for xx in 0..w {
            let o = &mut out[(y * w + xx) * cout..(y * w + xx + 1) * cout];
            o.copy_from_slice(&cv.b);
            for ky in 0..3 {
                let yy = y + ky;
                if yy == 0 || yy > h {
                    continue;
                }
                for kx in 0..3 {
                    let xs = xx + kx;
                    if xs == 0 || xs > w {
                        continue;
                    }
                    let inp = &x[((yy - 1) * w + xs - 1) * cin..((yy - 1) * w + xs) * cin];
                    let k = ky * 3 + kx;
                    for (c, &v) in inp.iter().enumerate() {
                        if v == 0.0 {
                            continue;
                        }
                        let wr = &cv.w[(k * cin + c) * cout..(k * cin + c + 1) * cout];
                        for (ov, &wv) in o.iter_mut().zip(wr) {
                            *ov += v * wv;
                        }
                    }
                }
            }
            for v in o.iter_mut() {
                if *v < 0.0 {
                    *v = 0.0;
                }
            }
        }
    }
    out
}

fn maxpool(x: &[f32], h: usize, w: usize, c: usize, ph: usize, pw: usize) -> Vec<f32> {
    let (oh, ow) = (h / ph, w / pw);
    let mut out = vec![f32::MIN; oh * ow * c];
    for y in 0..oh * ph {
        for xx in 0..ow * pw {
            let dst = ((y / ph) * ow + xx / pw) * c;
            let src = (y * w + xx) * c;
            for k in 0..c {
                let v = x[src + k];
                if v > out[dst + k] {
                    out[dst + k] = v;
                }
            }
        }
    }
    out
}

#[inline]
fn sigmoid(v: f32) -> f32 {
    1.0 / (1.0 + (-v).exp())
}

fn gru_dir(x: &[f32], t: usize, nin: usize, g: &GruDir, hid: usize, rev: bool, out: &mut [f32], stride: usize, off: usize) {
    let h3 = 3 * hid;
    // input projections for all steps
    let mut gi = vec![0.0f32; t * h3];
    for s in 0..t {
        let row = &mut gi[s * h3..(s + 1) * h3];
        row.copy_from_slice(&g.bih);
        for (k, &v) in x[s * nin..(s + 1) * nin].iter().enumerate() {
            if v == 0.0 {
                continue;
            }
            let wr = &g.wih[k * h3..(k + 1) * h3];
            for (o, &wv) in row.iter_mut().zip(wr) {
                *o += v * wv;
            }
        }
    }
    let mut h = vec![0.0f32; hid];
    let mut gh = vec![0.0f32; h3];
    for i in 0..t {
        let s = if rev { t - 1 - i } else { i };
        gh.copy_from_slice(&g.bhh);
        for (k, &v) in h.iter().enumerate() {
            let wr = &g.whh[k * h3..(k + 1) * h3];
            for (o, &wv) in gh.iter_mut().zip(wr) {
                *o += v * wv;
            }
        }
        let gis = &gi[s * h3..(s + 1) * h3];
        for j in 0..hid {
            let r = sigmoid(gis[j] + gh[j]);
            let z = sigmoid(gis[hid + j] + gh[hid + j]);
            let n = (gis[2 * hid + j] + r * gh[2 * hid + j]).tanh();
            h[j] = (1.0 - z) * n + z * h[j];
        }
        out[s * stride + off..s * stride + off + hid].copy_from_slice(&h);
    }
}

fn bigru(x: &[f32], t: usize, g: &Bigru, hid: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; t * 2 * hid];
    gru_dir(x, t, g.nin, &g.f, hid, false, &mut out, 2 * hid, 0);
    gru_dir(x, t, g.nin, &g.r, hid, true, &mut out, 2 * hid, hid);
    out
}
