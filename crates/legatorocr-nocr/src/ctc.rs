//! CTC decoding of the per-column log-probabilities.

/// One decoded character with the columns it occupies.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub ch: char,
    /// First and last column (inclusive) where this character was the best class.
    pub c0: usize,
    pub c1: usize,
    /// Probability of the character at its strongest column.
    pub p: f32,
    /// Best competing character at that column and its probability.
    pub alt: Option<(char, f32)>,
}

/// Best-path decoding: collapse repeats, drop blanks.
pub fn greedy(lp: &[f32], t: usize, nc: usize, charset: &[char]) -> Vec<Glyph> {
    let mut out: Vec<Glyph> = Vec::new();
    let mut prev = 0usize;
    for x in 0..t {
        let row = &lp[x * nc..(x + 1) * nc];
        let (k, &best) = row
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap();
        let p = best.exp();
        if k != 0 {
            let alt = || {
                row.iter()
                    .enumerate()
                    .filter(|&(i, _)| i != k && i != 0)
                    .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .and_then(|(i, &v)| charset.get(i - 1).map(|&c| (c, v.exp())))
            };
            if k == prev {
                if let Some(g) = out.last_mut() {
                    g.c1 = x;
                    if p > g.p {
                        g.p = p;
                        g.alt = alt();
                    }
                }
            } else if let Some(&ch) = charset.get(k - 1) {
                out.push(Glyph { ch, c0: x, c1: x, p, alt: alt() });
            }
        }
        prev = k;
    }
    out
}
