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
    (0..s.len() / 2)
        .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok())
        .collect()
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
    while k < n - k {
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
    if n == 0 || m >= n || path.iter().any(|h| h.len() != leaf.len()) {
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

/// Why a proof was turned down, or why it could not be judged at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProofError {
    /// Sizes outside the domain (for example an old size of 0 or larger than the new one): a
    /// mistake in the question, not a bad proof.
    BadInput(String),
    /// No proof elements where some are needed.
    Empty,
    /// The elements ran out before the roots could be rebuilt.
    TooShort,
    /// Elements were left over after both roots were rebuilt.
    TooLong,
    /// An element (or a root) does not have the length of a hash.
    HashLength,
    /// The rebuilt old or new root differs from the given one.
    RootMismatch,
}

impl std::fmt::Display for ProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProofError::BadInput(s) => write!(f, "bad input: {s}"),
            ProofError::Empty => f.write_str("the proof is empty"),
            ProofError::TooShort => f.write_str("the proof is too short"),
            ProofError::TooLong => f.write_str("the proof has elements left over"),
            ProofError::HashLength => f.write_str("a proof element has the wrong length"),
            ProofError::RootMismatch => f.write_str("a rebuilt root differs from the given one"),
        }
    }
}

impl std::error::Error for ProofError {}

/// Proof that the tree over the first `m` leaves is the unchanged left part of the tree over
/// all `leaves` (0 < m <= n). Bottom-up; empty when `m == n`. The old root itself is left out
/// when the old tree is a complete subtree at the left edge, because the verifier has it.
pub fn consistency_proof(leaves: &[Hash], m: usize) -> std::result::Result<Vec<Hash>, ProofError> {
    let n = leaves.len();
    if m == 0 || m > n {
        return Err(ProofError::BadInput(format!(
            "old size {m} must be between 1 and the new size {n}"
        )));
    }
    Ok(consistency_part(leaves, m, true))
}

/// `whole`: the old tree is exactly the left edge of this subtree's own parent chain, so its
/// root is already known to the verifier when sizes meet.
fn consistency_part(leaves: &[Hash], m: usize, whole: bool) -> Vec<Hash> {
    let n = leaves.len();
    if m == n {
        return if whole {
            Vec::new()
        } else {
            vec![root(leaves)]
        };
    }
    let k = split(n);
    if m <= k {
        let mut p = consistency_part(&leaves[..k], m, whole);
        p.push(root(&leaves[k..]));
        p
    } else {
        let mut p = consistency_part(&leaves[k..], m - k, false);
        p.push(root(&leaves[..k]));
        p
    }
}

/// Rebuild (old root, new root) of a tree of `n` leaves whose first `m` form the old tree,
/// taking proof elements from the back (the top of the tree is met first).
fn rebuild(
    m: usize,
    n: usize,
    whole: bool,
    old_root: &[u8],
    proof: &mut &[Hash],
) -> std::result::Result<(Hash, Hash), ProofError> {
    let take = |proof: &mut &[Hash]| -> std::result::Result<Hash, ProofError> {
        let (last, rest) = proof.split_last().ok_or(ProofError::TooShort)?;
        *proof = rest;
        Ok(last.clone())
    };
    if m == n {
        let h = if whole {
            old_root.to_vec()
        } else {
            take(proof)?
        };
        return Ok((h.clone(), h));
    }
    let k = split(n);
    if m <= k {
        let right = take(proof)?;
        let (o, nw) = rebuild(m, k, whole, old_root, proof)?;
        Ok((o, node(&nw, &right)))
    } else {
        let left = take(proof)?;
        let (o, nw) = rebuild(m - k, n - k, false, old_root, proof)?;
        Ok((node(&left, &o), node(&left, &nw)))
    }
}

/// Accept only if `proof` rebuilds both `old_root` (size `m`) and `new_root` (size `n`) and is
/// used up exactly. Needs 0 < m < n; the equal-size case is [`check_consistency`].
pub fn verify_consistency(
    m: usize,
    n: usize,
    old_root: &[u8],
    new_root: &[u8],
    proof: &[Hash],
) -> std::result::Result<(), ProofError> {
    if m == 0 || m > n {
        return Err(ProofError::BadInput(format!(
            "old size {m} must be between 1 and the new size {n}"
        )));
    }
    if m == n {
        return Err(ProofError::Empty);
    }
    if proof.is_empty() {
        return Err(ProofError::Empty);
    }
    if old_root.len() != new_root.len() || proof.iter().any(|h| h.len() != old_root.len()) {
        return Err(ProofError::HashLength);
    }
    let mut rest = proof;
    let (o, nw) = rebuild(m, n, true, old_root, &mut rest)?;
    if !rest.is_empty() {
        return Err(ProofError::TooLong);
    }
    if o != old_root || nw != new_root {
        return Err(ProofError::RootMismatch);
    }
    Ok(())
}

/// [`verify_consistency`] plus the equal-size case, where the roots are simply compared.
pub fn check_consistency(
    m: usize,
    n: usize,
    old_root: &[u8],
    new_root: &[u8],
    proof: &[Hash],
) -> std::result::Result<(), ProofError> {
    if m == n && m > 0 {
        return if old_root == new_root && proof.is_empty() {
            Ok(())
        } else {
            Err(ProofError::RootMismatch)
        };
    }
    verify_consistency(m, n, old_root, new_root, proof)
}

/// Like [`parse_sums`] but the leaves stay in the order of the lines (insertion order): a
/// collection that only grows at the end keeps its old leaves in place, which consistency
/// proofs depend on. Sorting would move new files into the middle and change old roots.
pub fn parse_sums_in_order(text: &str) -> Result<Vec<(String, String)>> {
    let mut v = parse_sums(text)?;
    let order: std::collections::HashMap<String, usize> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .filter_map(|(i, l)| {
            let name = l.split_once("  ").or_else(|| l.split_once(" *"))?.1;
            Some((name.to_string(), i))
        })
        .collect();
    v.sort_by_key(|(n, _)| order.get(n).copied().unwrap_or(usize::MAX));
    Ok(v)
}

/// Check a consistency proof given as hex strings.
pub fn check_consistency_hex(
    m: usize,
    n: usize,
    old_root: &str,
    new_root: &str,
    path_hex: &[String],
) -> std::result::Result<(), ProofError> {
    let (Some(o), Some(nw)) = (from_hex(old_root), from_hex(new_root)) else {
        return Err(ProofError::HashLength);
    };
    let path = path_hex
        .iter()
        .map(|h| from_hex(h))
        .collect::<Option<Vec<_>>>()
        .ok_or(ProofError::HashLength)?;
    check_consistency(m, n, &o, &nw, &path)
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
pub fn check(
    name: &str,
    file_hash: &str,
    m: usize,
    n: usize,
    path_hex: &[String],
    expected: &str,
) -> bool {
    let Some(path) = path_hex
        .iter()
        .map(|h| from_hex(h))
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };
    root_from_proof(&leaf_hash(name, file_hash), m, n, &path)
        .is_some_and(|r| hex(&r).eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: usize) -> Vec<Hash> {
        (0..n)
            .map(|i| leaf_hash(&format!("f{i}"), &"ab".repeat(32)))
            .collect()
    }

    #[test]
    fn rfc6962_known_values() {
        // empty tree = SHA-256 of nothing; a single empty leaf = SHA-256(0x00)
        assert_eq!(
            hex(&root(&[])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hex(&sha256(&[0u8])),
            "6e340b9cffb37a989ca544e6bb780a2c78901d3fb33738768511a30617afa01d"
        );
    }

    #[test]
    fn every_leaf_of_every_small_tree_has_a_valid_proof_and_tampering_fails() {
        for n in 1..=17 {
            let l = leaves(n);
            let r = root(&l);
            for m in 0..n {
                let p = proof(&l, m);
                assert_eq!(
                    root_from_proof(&l[m], m, n, &p).as_deref(),
                    Some(r.as_slice()),
                    "n={n} m={m}"
                );
                let other = leaf_hash("forged", &"cd".repeat(32));
                assert_ne!(
                    root_from_proof(&other, m, n, &p).as_deref(),
                    Some(r.as_slice())
                );
                if n > 1 {
                    assert_ne!(
                        root_from_proof(&l[m], (m + 1) % n, n, &p).as_deref(),
                        Some(r.as_slice()),
                        "wrong position n={n} m={m}"
                    );
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

    fn raw_leaves(n: usize) -> Vec<Hash> {
        (0..n).map(|i| sha256(&[0u8, i as u8])).collect()
    }

    #[test]
    fn small_trees_have_the_shapes_of_the_definition() {
        assert!(proof(&raw_leaves(1), 0).is_empty());
        let l = raw_leaves(1);
        assert_eq!(root(&l), l[0]);
        let l2 = raw_leaves(2);
        for m in 0..2 {
            assert_eq!(proof(&l2, m), vec![l2[1 - m].clone()]);
        }
        // 7 leaves split 4 + 3
        let l7 = raw_leaves(7);
        assert_eq!(root(&l7), node(&root(&l7[..4]), &root(&l7[4..])));
        for m in 0..7 {
            let p = proof(&l7, m);
            assert_eq!(
                root_from_proof(&l7[m], m, 7, &p).as_deref(),
                Some(root(&l7).as_slice())
            );
            let mut bad = l7[m].clone();
            bad[0] ^= 1;
            assert_ne!(
                root_from_proof(&bad, m, 7, &p).as_deref(),
                Some(root(&l7).as_slice())
            );
            for i in 0..p.len() {
                let mut q = p.clone();
                q[i][0] ^= 1;
                assert_ne!(
                    root_from_proof(&l7[m], m, 7, &q).as_deref(),
                    Some(root(&l7).as_slice())
                );
            }
        }
        assert!(root_from_proof(&l7[0], 7, 7, &proof(&l7, 0)).is_none());
    }

    #[test]
    fn consistency_3_to_7_and_4_to_8() {
        let l = raw_leaves(7);
        let p = consistency_proof(&l, 3).unwrap();
        assert!(p.len() <= 4);
        let (o, n) = (root(&l[..3]), root(&l));
        assert_eq!(verify_consistency(3, 7, &o, &n, &p), Ok(()));
        assert_eq!(
            verify_consistency(3, 7, &o, &n, &[]),
            Err(ProofError::Empty)
        );
        assert!(verify_consistency(3, 7, &root(&l[..2]), &n, &p).is_err());
        let l8 = raw_leaves(8);
        let p = consistency_proof(&l8, 4).unwrap();
        let old = root(&l8[..4]);
        assert!(!p.contains(&old), "the old root is not in the proof");
        assert_eq!(p.len(), 1);
        assert_eq!(verify_consistency(4, 8, &old, &root(&l8), &p), Ok(()));
    }

    #[test]
    fn equal_sizes_and_bad_input() {
        let l = raw_leaves(5);
        let r = root(&l);
        assert!(consistency_proof(&l, 5).unwrap().is_empty());
        assert!(verify_consistency(5, 5, &r, &r, &[]).is_err());
        assert_eq!(check_consistency(5, 5, &r, &r, &[]), Ok(()));
        assert!(check_consistency(5, 5, &r, &root(&l[..4]), &[]).is_err());
        assert!(matches!(
            consistency_proof(&l, 0),
            Err(ProofError::BadInput(_))
        ));
        assert!(matches!(
            consistency_proof(&l, 6),
            Err(ProofError::BadInput(_))
        ));
        assert!(matches!(
            verify_consistency(0, 5, &r, &r, std::slice::from_ref(&r)),
            Err(ProofError::BadInput(_))
        ));
        // an element of the wrong length is refused
        let p = consistency_proof(&l, 2).unwrap();
        let mut short = p.clone();
        short[0].pop();
        assert_eq!(
            verify_consistency(2, 5, &root(&l[..2]), &r, &short),
            Err(ProofError::HashLength)
        );
    }

    #[test]
    fn every_pair_up_to_17_verifies_and_altered_proofs_fail() {
        for n in 2..=17 {
            let l = raw_leaves(n);
            let new = root(&l);
            for m in 1..n {
                let old = root(&l[..m]);
                let p = consistency_proof(&l, m).unwrap();
                let bound = (usize::BITS - (n - 1).leading_zeros()) as usize + 1; // ceil(log2 n) + 1
                assert!(p.len() <= bound, "n={n} m={m} len={}", p.len());
                assert_eq!(verify_consistency(m, n, &old, &new, &p), Ok(()), "{m}->{n}");
                for i in 0..p.len() {
                    let mut q = p.clone();
                    q[i][3] ^= 0x10;
                    assert!(verify_consistency(m, n, &old, &new, &q).is_err());
                }
                let mut extra = p.clone();
                extra.push(new.clone());
                assert!(verify_consistency(m, n, &old, &new, &extra).is_err());
                assert!(verify_consistency(m, n, &old, &new, &p[1..]).is_err());
            }
        }
    }

    #[test]
    fn growing_a_collection_keeps_old_leaves_in_order() {
        let h = |c: char| c.to_string().repeat(64);
        let old = format!("{}  z.pdf\n{}  a.pdf\n", h('1'), h('2'));
        let new = format!("{old}{}  m.pdf\n", h('3'));
        let (o, n) = (
            parse_sums_in_order(&old).unwrap(),
            parse_sums_in_order(&new).unwrap(),
        );
        assert_eq!(n[0].0, "z.pdf");
        assert_eq!(n[2].0, "m.pdf");
        let (lo, ln) = (leaves_of(&o), leaves_of(&n));
        let p = consistency_proof(&ln, 2).unwrap();
        assert_eq!(verify_consistency(2, 3, &root(&lo), &root(&ln), &p), Ok(()));
        let hexp: Vec<String> = p.iter().map(|x| hex(x)).collect();
        assert_eq!(
            check_consistency_hex(2, 3, &hex(&root(&lo)), &hex(&root(&ln)), &hexp),
            Ok(())
        );
    }

    #[test]
    fn split_does_not_overflow_for_huge_sizes() {
        assert_eq!(split(usize::MAX), 1usize << (usize::BITS - 1));
        assert_eq!(split(2), 1);
        assert_eq!(split(3), 2);
        assert_eq!(split(8), 4);
        assert_eq!(split(9), 8);
    }
}
