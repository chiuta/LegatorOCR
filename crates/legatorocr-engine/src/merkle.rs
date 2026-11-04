//! Merkle tree over a `SHA256SUMS.txt`, built as in RFC 6962 (Certificate Transparency; RFC 6962 has since been
//! obsoleted by RFC 9162, CT v2.0, whose hashing was not compared here):
//! leaf = SHA-256(0x00 ‖ name ‖ 0x00 ‖ file hash), node = SHA-256(0x01 ‖ left ‖ right), the
//! tree split at the largest power of two below the leaf count. One short root then stands for a
//! whole collection: publish or timestamp the root (e.g. with an RFC 3161 service of your
//! choice, which this program does not call) and any file can later be proven to belong to it
//! with a short audit path.

use anyhow::{bail, Result};
use legatorocr_pdf::crypt::sha256;

pub type Hash = Vec<u8>;

pub fn hex(h: &[u8]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

fn from_hex(s: &str) -> Option<Hash> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return None;
    }
    (0..s.len() / 2).map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}

pub fn leaf_hash(name: &str, file_hash_hex: &str) -> Hash {
    let mut d = vec![0u8];
    d.extend_from_slice(name.as_bytes());
    d.push(0);
    d.extend_from_slice(file_hash_hex.to_ascii_lowercase().as_bytes());
    sha256(&d)
}

fn node(l: &[u8], r: &[u8]) -> Hash {
    let mut d = vec![1u8];
    d.extend_from_slice(l);
    d.extend_from_slice(r);
    sha256(&d)
}

fn split(n: usize) -> usize {
    // largest power of two strictly below n (n >= 2)
    let mut k = 1;
    while k * 2 < n {
        k *= 2;
    }
    k
}

/// Root of the tree over already hashed leaves (the empty tree hashes the empty string).
pub fn root(leaves: &[Hash]) -> Hash {
    match leaves.len() {
        0 => sha256(b""),
        1 => leaves[0].clone(),
        n => {
            let k = split(n);
            node(&root(&leaves[..k]), &root(&leaves[k..]))
        }
    }
}

/// Audit path for leaf `m`: the sibling hashes from the leaf up to the root.
pub fn proof(leaves: &[Hash], m: usize) -> Vec<Hash> {
    let n = leaves.len();
    if n <= 1 {
        return Vec::new();
    }
    let k = split(n);
    if m < k {
        let mut p = proof(&leaves[..k], m);
        p.push(root(&leaves[k..]));
        p
    } else {
        let mut p = proof(&leaves[k..], m - k);
        p.push(root(&leaves[..k]));
        p
    }
}

/// Root implied by a leaf hash, its index `m` of `n` leaves and its audit path.
pub fn root_from_proof(leaf: &[u8], m: usize, n: usize, path: &[Hash]) -> Option<Hash> {
    if n == 0 || m >= n {
        return None;
    }
    if n == 1 {
        return path.is_empty().then(|| leaf.to_vec());
    }
    let (last, rest) = path.split_last()?;
    let k = split(n);
    if m < k {
        Some(node(&root_from_proof(leaf, m, k, rest)?, last))
    } else {
        Some(node(last, &root_from_proof(leaf, m - k, n - k, rest)?))
    }
}

/// `(name, hash)` pairs from `sha256sum`-style text, sorted by name. Escaped names are refused
/// rather than guessed at.
pub fn parse_sums(text: &str) -> Result<Vec<(String, String)>> {
    let mut v = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with('\\') {
            bail!("a name with special characters ({line:?}) is not supported here");
        }
        let Some((h, name)) = line.split_once("  ").or_else(|| line.split_once(" *")) else {
            bail!("malformed line: {line:?}");
        };
        if h.len() != 64 || from_hex(h).is_none() {
            bail!("not a SHA-256 digest: {h:?}");
        }
        v.push((name.to_string(), h.to_ascii_lowercase()));
    }
    v.sort();
    if v.windows(2).any(|w| w[0].0 == w[1].0) {
        bail!("the same file name is listed twice");
    }
    Ok(v)
}

pub fn leaves_of(entries: &[(String, String)]) -> Vec<Hash> {
    entries.iter().map(|(n, h)| leaf_hash(n, h)).collect()
}

/// Check an audit path given as hex strings against an expected root.
pub fn check(name: &str, file_hash: &str, m: usize, n: usize, path_hex: &[String], expected: &str) -> bool {
    let Some(path) = path_hex.iter().map(|h| from_hex(h)).collect::<Option<Vec<_>>>() else {
        return false;
    };
    root_from_proof(&leaf_hash(name, file_hash), m, n, &path)
        .is_some_and(|r| hex(&r).eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n).map(|i| leaf_hash(&format!("f{i}"), &"ab".repeat(32))).collect()
    }

    #[test]
    fn rfc6962_known_values() {
        // empty tree = SHA-256 of nothing; a single empty leaf = SHA-256(0x00)
        assert_eq!(hex(&root(&[])), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hex(&sha256(&[0u8])), "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d");
    }

    #[test]
    fn every_leaf_of_every_small_tree_has_a_valid_proof_and_tampering_fails() {
        for n in 1..=17 {
            let l = leaves(n);
            let r = root(&l);
            for m in 0..n {
                let p = proof(&l, m);
                assert_eq!(root_from_proof(&l[m], m, n, &p).as_deref(), Some(r.as_slice()), "n={n} m={m}");
                let other = leaf_hash("forged", &"cd".repeat(32));
                assert_ne!(root_from_proof(&other, m, n, &p).as_deref(), Some(r.as_slice()));
                if n > 1 {
                    assert_ne!(root_from_proof(&l[m], (m + 1) % n, n, &p).as_deref(), Some(r.as_slice()), "wrong position n={n} m={m}");
                }
            }
        }
    }

    #[test]
    fn sums_parse_and_the_hex_check_works() {
        let h1 = "11".repeat(32);
        let h2 = "22".repeat(32);
        let e = parse_sums(&format!("{h2}  b.pdf\n{h1}  a.pdf\n")).unwrap();
        assert_eq!(e[0].0, "a.pdf");
        let l = leaves_of(&e);
        let r = hex(&root(&l));
        let p: Vec<String> = proof(&l, 1).iter().map(|h| hex(h)).collect();
        assert!(check("b.pdf", &h2, 1, 2, &p, &r));
        assert!(!check("b.pdf", &h1, 1, 2, &p, &r));
        assert!(parse_sums(&format!("{h1}  a\n{h1}  a\n")).is_err());
        assert!(parse_sums("zz  a").is_err());
    }
}
