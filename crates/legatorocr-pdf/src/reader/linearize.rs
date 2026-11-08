//! Linearize a finished PDF ("Fast Web View", ISO 32000-1 Annex F): the first page and what it
//! needs come first, so a viewer can show page 1 before the rest has arrived, and a hint stream
//! tells it where every other page is. Works on the files this program writes (classic
//! cross-reference table, no object streams); an encrypted or already compact file is refused.
//! Every object is its own shared-object group, as qpdf does; offsets are exact because the
//! cross-reference tables and the hint stream have a size that does not depend on the offsets.

use super::encrypt::{hex, Ctx, Hex};
use super::{fail, PdfDoc, Val, P, R};
use crate::crypt::md5;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Most significant bit first.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn new() -> Bits {
        Bits {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }
    fn put(&mut self, v: u64, bits: u32) {
        for i in (0..bits).rev() {
            self.acc = self.acc << 1 | (v >> i & 1);
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }
    fn flush(&mut self) {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
            self.acc = 0;
            self.n = 0;
        }
    }
}

/// Bits needed to write `v`.
fn nbits(v: u64) -> u32 {
    64 - v.leading_zeros()
}

fn refs_of(v: &Val, skip_parent: bool, out: &mut Vec<u32>) {
    match v {
        Val::Ref(n) => out.push(*n),
        Val::Arr(a) => a.iter().for_each(|x| refs_of(x, false, out)),
        Val::Dict(d) | Val::Stream(d, _, _) => {
            let mut keys: Vec<_> = d.keys().collect();
            keys.sort();
            for k in keys {
                if skip_parent && k == "Parent" {
                    continue;
                }
                refs_of(&d[k], false, out);
            }
        }
        _ => {}
    }
}

fn collect_pages(
    doc: &PdfDoc,
    node: u32,
    depth: u32,
    pages: &mut Vec<u32>,
    nodes: &mut HashSet<u32>,
) -> bool {
    if depth > 64 || nodes.contains(&node) || pages.contains(&node) {
        return false;
    }
    let Some(Val::Dict(d)) = doc.get(node) else {
        return false;
    };
    let kids = match d.get("Kids") {
        Some(Val::Arr(a)) => Some(a.clone()),
        Some(Val::Ref(r)) => match doc.get(*r) {
            Some(Val::Arr(a)) => Some(a),
            _ => None,
        },
        _ => None,
    };
    match kids {
        Some(k) => {
            nodes.insert(node);
            for x in k {
                if let Val::Ref(r) = x {
                    collect_pages(doc, r, depth + 1, pages, nodes);
                }
            }
        }
        None => pages.push(node),
    }
    true
}

pub fn linearize(data: &[u8]) -> R<Vec<u8>> {
    linearize_with(data, None, &mut |_| {})
}

/// Like [`linearize`], and with `protect` the output is encrypted (AES-256) as it is written: the
/// Encrypt dictionary follows the catalog, every string and stream (the hint stream too) is
/// encrypted, the layout offsets are computed on the encrypted sizes.
pub fn linearize_with(
    data: &[u8],
    protect: Option<&super::encrypt::Protect>,
    rand: &mut dyn FnMut(&mut [u8]),
) -> R<Vec<u8>> {
    let setup = protect.map(|p| super::encrypt::setup_for(p, rand));
    let doc = PdfDoc::open(data.to_vec())?;
    let Some(t) = data.windows(7).rposition(|w| w == b"trailer") else {
        return fail("only files with a classic cross-reference table can be linearized");
    };
    let Some(Val::Dict(tr)) = P::new(data, t + 7).val(0) else {
        return fail("the trailer cannot be read");
    };
    if tr.contains_key("Encrypt") {
        return fail("the file is already encrypted");
    }
    let size = match tr.get("Size") {
        Some(Val::Int(n)) if (2..=5_000_000).contains(n) => *n as u32,
        _ => return fail("the trailer has no /Size"),
    };
    let Some(Val::Ref(root)) = tr.get("Root").cloned() else {
        return fail("the trailer has no /Root");
    };
    let info = match tr.get("Info") {
        Some(Val::Ref(n)) => Some(*n),
        _ => None,
    };
    let Some(Val::Dict(cat)) = doc.get(root) else {
        return fail("the catalog cannot be read");
    };
    let Some(Val::Ref(pages_root)) = cat.get("Pages").cloned() else {
        return fail("the catalog has no /Pages");
    };
    let mut pages = Vec::new();
    let mut nodes = HashSet::new();
    collect_pages(&doc, pages_root, 0, &mut pages, &mut nodes);
    if pages.is_empty() {
        return fail("the file has no pages");
    }
    let page_set: HashSet<u32> = pages.iter().copied().collect();
    // objects each page needs, the page object first
    let reach: Vec<Vec<u32>> = pages
        .iter()
        .map(|&p| {
            let mut seen: HashSet<u32> = HashSet::from([p]);
            let mut order = vec![p];
            let mut stack: Vec<u32> = Vec::new();
            let mut first = Vec::new();
            if let Some(v) = doc.get(p) {
                refs_of(&v, true, &mut first);
            }
            first.reverse();
            stack.extend(first);
            while let Some(n) = stack.pop() {
                if n == 0
                    || n >= size
                    || n == root
                    || nodes.contains(&n)
                    || page_set.contains(&n)
                    || !seen.insert(n)
                {
                    continue;
                }
                order.push(n);
                let mut r = Vec::new();
                if let Some(v) = doc.get(n) {
                    refs_of(&v, false, &mut r);
                }
                r.reverse();
                stack.extend(r);
            }
            order
        })
        .collect();
    let f1 = &reach[0];
    let f1set: HashSet<u32> = f1.iter().copied().collect();
    let mut users: HashMap<u32, usize> = HashMap::new();
    for r in reach.iter().skip(1) {
        for &o in r.iter().skip(1) {
            if !f1set.contains(&o) {
                *users.entry(o).or_default() += 1;
            }
        }
    }
    let mut private: Vec<Vec<u32>> = vec![Vec::new()];
    let mut shared: Vec<u32> = Vec::new();
    let mut placed: HashSet<u32> = f1set.clone();
    placed.insert(root);
    for r in reach.iter().skip(1) {
        let mut mine = Vec::new();
        for &o in r.iter().skip(1) {
            if f1set.contains(&o) {
                continue;
            }
            if users[&o] == 1 {
                mine.push(o);
            } else if placed.insert(o) {
                shared.push(o);
            }
        }
        for &o in &mine {
            placed.insert(o);
        }
        private.push(mine);
    }
    for &p in pages.iter().skip(1) {
        placed.insert(p);
    }
    let others: Vec<u32> = (1..size).filter(|n| !placed.contains(n)).collect();
    // main section order: remaining pages (page object, its private objects), shared, others
    let mut main_order: Vec<u32> = Vec::new();
    for (i, &p) in pages.iter().enumerate().skip(1) {
        main_order.push(p);
        main_order.extend(&private[i]);
    }
    let shared_start = main_order.len();
    main_order.extend(&shared);
    main_order.extend(&others);
    let m = main_order.len() as u32;
    let (lin_n, cat_n, hint_n) = (m + 1, m + 2, m + 3);
    // the Encrypt dictionary sits between the catalog and the hint stream
    let enc_n = setup.as_ref().map(|_| hint_n + 1);
    let shift = enc_n.is_some() as u32;
    let mut map = vec![0u32; size as usize];
    for (i, &o) in main_order.iter().enumerate() {
        map[o as usize] = i as u32 + 1;
    }
    map[root as usize] = cat_n;
    for (k, &o) in f1.iter().enumerate() {
        map[o as usize] = hint_n + 1 + shift + k as u32;
    }
    let total = hint_n + 1 + shift + f1.len() as u32;
    // object bytes under their new numbers
    let mut cx = Ctx {
        key: setup.as_ref().map(|s| s.key),
        rand,
        map: Some(&map),
    };
    let mut body = |old: u32| -> Vec<u8> {
        let mut b = Vec::new();
        let _ = writeln!(Hex(&mut b), "{} 0 obj", map_of(&cx, old));
        match doc.get(old) {
            Some(Val::Stream(d, a, e)) => {
                let raw = cx.enc(data.get(a..e).unwrap_or_default());
                cx.dict(&d, Some(raw.len()), &mut b);
                b.extend_from_slice(b"\nstream\n");
                b.extend_from_slice(&raw);
                b.extend_from_slice(b"\nendstream");
            }
            Some(v) => cx.ser(&v, &mut b),
            None => b.extend_from_slice(b"null"),
        }
        b.extend_from_slice(b"\nendobj\n");
        b
    };
    let cat_b = body(root);
    let f1_b: Vec<Vec<u8>> = f1.iter().map(|&o| body(o)).collect();
    let main_b: Vec<Vec<u8>> = main_order.iter().map(|&o| body(o)).collect();
    // sizes of the page groups
    let npages = pages.len();
    let page_len: Vec<usize> = (0..npages)
        .map(|i| {
            if i == 0 {
                f1_b.iter().map(Vec::len).sum()
            } else {
                // page object and privates are consecutive in main_order
                let start = main_order.iter().position(|&o| o == pages[i]).unwrap_or(0);
                main_b[start..start + 1 + private[i].len()]
                    .iter()
                    .map(Vec::len)
                    .sum()
            }
        })
        .collect();
    let nobj: Vec<usize> = (0..npages)
        .map(|i| {
            if i == 0 {
                f1.len()
            } else {
                1 + private[i].len()
            }
        })
        .collect();
    // shared-object table: first-page objects, then the shared region
    let mut idx: HashMap<u32, usize> = HashMap::new();
    for (k, &o) in f1.iter().enumerate() {
        idx.insert(o, k);
    }
    for (j, &o) in shared.iter().enumerate() {
        idx.insert(o, f1.len() + j);
    }
    let nshared_total = f1.len() + shared.len();
    let page_shared: Vec<Vec<usize>> = (0..npages)
        .map(|i| {
            if i == 0 {
                return Vec::new();
            }
            let mut v: Vec<usize> = reach[i]
                .iter()
                .skip(1)
                .filter_map(|o| idx.get(o).copied())
                .collect();
            v.sort_unstable();
            v
        })
        .collect();
    let group_len: Vec<usize> = f1_b
        .iter()
        .map(Vec::len)
        .chain(
            main_b[shared_start..shared_start + shared.len()]
                .iter()
                .map(Vec::len),
        )
        .collect();
    // fixed-size blocks
    let header: &[u8] = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n";
    let lin_text = |l: usize, ho: usize, hl: usize, e: usize, tt: usize| -> Vec<u8> {
        let mut d = format!(
            "<< /Linearized 1 /L {l} /H [ {ho} {hl} ] /O {} /E {e} /N {npages} /T {tt} >>",
            map[pages[0] as usize]
        );
        while d.len() < 130 {
            d.push(' ');
        }
        format!("{lin_n} 0 obj\n{d}\nendobj\n").into_bytes()
    };
    let lin_len = lin_text(0, 0, 0, 0, 0).len();
    let id = hex(&md5(data));
    let cnt_f = 2 + shift + f1.len() as u32 + 1; // lin, catalog, [encrypt], hint, first page objects
    let enc_s = enc_n.map_or(String::new(), |n| format!(" /Encrypt {n} 0 R"));
    let enc_b: Vec<u8> = match (&setup, protect) {
        (Some(s), Some(p)) => {
            super::encrypt::encrypt_object(enc_n.unwrap_or(0), p.allowed, s).into_bytes()
        }
        _ => Vec::new(),
    };
    let info_s = info.map_or(String::new(), |n| format!(" /Info {} 0 R", map[n as usize]));
    let first_trailer = |prev: usize| {
        format!(
            "trailer << /Size {total} /Root {cat_n} 0 R{info_s}{enc_s} /Prev {prev:<10} /ID [{id} {id}] >>\nstartxref\n0\n%%EOF\n"
        )
    };
    let xref1_head = format!("xref\n{lin_n} {cnt_f}\n");
    let xref1_len = xref1_head.len() + 20 * cnt_f as usize + first_trailer(0).len();
    // hint stream: tables first (their size depends on the lengths only)
    let hint_data = |first_page_off: usize, first_shared_off: usize| -> (Vec<u8>, usize) {
        let min_n = *nobj.iter().min().unwrap_or(&0);
        let max_n = *nobj.iter().max().unwrap_or(&0);
        let min_l = *page_len.iter().min().unwrap_or(&0);
        let max_l = *page_len.iter().max().unwrap_or(&0);
        let b_n = nbits((max_n - min_n) as u64);
        let b_l = nbits((max_l - min_l) as u64);
        let max_sh = page_shared.iter().map(Vec::len).max().unwrap_or(0);
        let b_sh = nbits(max_sh as u64);
        let b_id = nbits((nshared_total.saturating_sub(1)) as u64);
        let mut w = Bits::new();
        w.put(min_n as u64, 32);
        w.put(first_page_off as u64, 32);
        w.put(b_n as u64, 16);
        w.put(min_l as u64, 32);
        w.put(b_l as u64, 16);
        w.put(0, 32);
        w.put(0, 16);
        w.put(min_l as u64, 32);
        w.put(b_l as u64, 16);
        w.put(b_sh as u64, 16);
        w.put(b_id as u64, 16);
        w.put(0, 16);
        w.put(4, 16);
        for &n in &nobj {
            w.put((n - min_n) as u64, b_n);
        }
        w.flush();
        for &l in &page_len {
            w.put((l - min_l) as u64, b_l);
        }
        w.flush();
        for s in &page_shared {
            w.put(s.len() as u64, b_sh);
        }
        w.flush();
        for s in &page_shared {
            for &i in s {
                w.put(i as u64, b_id);
            }
        }
        w.flush();
        // numerators and content offsets are zero bits wide
        for &l in &page_len {
            w.put((l - min_l) as u64, b_l);
        }
        w.flush();
        let shared_at = w.out.len();
        let min_g = *group_len.iter().min().unwrap_or(&0);
        let max_g = *group_len.iter().max().unwrap_or(&0);
        let b_g = nbits((max_g - min_g) as u64);
        let (fo, fs) = if shared.is_empty() {
            (0u64, 0u64)
        } else {
            (map[shared[0] as usize] as u64, first_shared_off as u64)
        };
        w.put(fo, 32);
        w.put(fs, 32);
        w.put(f1.len() as u64, 32);
        w.put(nshared_total as u64, 32);
        w.put(0, 16);
        w.put(min_g as u64, 32);
        w.put(b_g as u64, 16);
        for &g in &group_len {
            w.put((g - min_g) as u64, b_g);
        }
        w.flush();
        for _ in &group_len {
            w.put(0, 1);
        }
        w.flush();
        (w.out, shared_at)
    };
    let hint_obj = |hd: &[u8], s: usize| -> Vec<u8> {
        // the payload is encrypted before it gets here (AES-CBC: IV + padded data)
        let mut b = format!(
            "{hint_n} 0 obj\n<< /S {s} /Length {} >>\nstream\n",
            hd.len()
        )
        .into_bytes();
        b.extend_from_slice(hd);
        b.extend_from_slice(b"\nendstream\nendobj\n");
        b
    };
    let (hd0, s0) = hint_data(0, 0);
    let hint_len = hint_obj(&cx.enc(&hd0), s0).len();
    // offsets
    let pos_lin = header.len();
    let pos_xref1 = pos_lin + lin_len;
    let pos_cat = pos_xref1 + xref1_len;
    let pos_enc = pos_cat + cat_b.len();
    let pos_hint = pos_enc + enc_b.len();
    let pos_f1 = pos_hint + hint_len;
    let mut offs = vec![0usize; total as usize];
    offs[lin_n as usize] = pos_lin;
    offs[cat_n as usize] = pos_cat;
    offs[hint_n as usize] = pos_hint;
    if let Some(n) = enc_n {
        offs[n as usize] = pos_enc;
    }
    let mut at = pos_f1;
    for (k, b) in f1_b.iter().enumerate() {
        offs[(hint_n + 1 + shift) as usize + k] = at;
        at += b.len();
    }
    let end_first = at;
    let mut first_shared_off = 0;
    for (k, b) in main_b.iter().enumerate() {
        if k == shared_start {
            first_shared_off = at;
        }
        offs[k + 1] = at;
        at += b.len();
    }
    let pos_xref2 = at;
    let xref2_head = format!("xref\n0 {}\n0000000000 65535 f \n", m + 1);
    let xref2_trailer = format!(
        "trailer << /Size {} /ID [{id} {id}]{enc_s} >>\nstartxref\n{pos_xref1}\n%%EOF\n",
        m + 1
    );
    let file_len = pos_xref2 + xref2_head.len() + 20 * m as usize + xref2_trailer.len();
    // offsets in the hint tables do not count the hint stream itself
    let (hd, s) = hint_data(pos_f1 - hint_len, first_shared_off.saturating_sub(hint_len));
    let hint_b = hint_obj(&cx.enc(&hd), s);
    debug_assert_eq!(hint_b.len(), hint_len);
    // write
    let mut out: Vec<u8> = Vec::with_capacity(file_len);
    out.extend_from_slice(header);
    // /T: the white space just before the first entry of the main table
    let t_off = pos_xref2 + format!("xref\n0 {}\n", m + 1).len() - 1;
    out.extend_from_slice(&lin_text(file_len, pos_hint, hint_len, end_first, t_off));
    out.extend_from_slice(xref1_head.as_bytes());
    for n in lin_n..total {
        let _ = writeln!(Hex(&mut out), "{:010} 00000 n ", offs[n as usize]);
    }
    out.extend_from_slice(first_trailer(pos_xref2).as_bytes());
    out.extend_from_slice(&cat_b);
    out.extend_from_slice(&enc_b);
    out.extend_from_slice(&hint_b);
    for b in &f1_b {
        out.extend_from_slice(b);
    }
    for b in &main_b {
        out.extend_from_slice(b);
    }
    out.extend_from_slice(xref2_head.as_bytes());
    for n in 1..=m {
        let _ = writeln!(Hex(&mut out), "{:010} 00000 n ", offs[n as usize]);
    }
    out.extend_from_slice(xref2_trailer.as_bytes());
    debug_assert_eq!(out.len(), file_len);
    Ok(out)
}

fn map_of(cx: &Ctx, old: u32) -> u32 {
    cx.map.map_or(old, |m| m[old as usize])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `pages` pages, each with its own content stream; all share one font; page 2 and 3 also
    /// share an extra object.
    pub(crate) fn sample(pages: usize) -> Vec<u8> {
        let mut objs: Vec<(u32, String)> = Vec::new();
        let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 10 + i * 2)).collect();
        objs.push((1, "<< /Type /Catalog /Pages 2 0 R >>".into()));
        objs.push((
            2,
            format!(
                "<< /Type /Pages /Kids [{}] /Count {pages} >>",
                kids.join(" ")
            ),
        ));
        objs.push((
            3,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        ));
        objs.push((4, "<< /Producer (t) >>".into()));
        objs.push((5, "<< /Shared 1 >>".into()));
        for i in 0..pages {
            let extra = if i == 1 || i == 2 {
                " /Extra 5 0 R"
            } else {
                ""
            };
            objs.push((
                10 + i as u32 * 2,
                format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R{extra} >>", 11 + i * 2),
            ));
            let c = format!("BT /F1 12 Tf 1 0 0 1 10 10 Tm (page {i}) Tj ET");
            objs.push((
                11 + i as u32 * 2,
                format!("<< /Length {} >>\nstream\n{c}\nendstream", c.len()),
            ));
        }
        let size = objs.iter().map(|o| o.0).max().unwrap() + 1;
        let mut o = b"%PDF-1.4\n".to_vec();
        let mut offs = vec![0usize; size as usize];
        for (n, b) in &objs {
            offs[*n as usize] = o.len();
            o.extend_from_slice(format!("{n} 0 obj\n{b}\nendobj\n").as_bytes());
        }
        let x = o.len();
        o.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f \n").as_bytes());
        for n in 1..size {
            if offs[n as usize] == 0 {
                o.extend_from_slice(b"0000000000 65535 f \n");
            } else {
                o.extend_from_slice(format!("{:010} 00000 n \n", offs[n as usize]).as_bytes());
            }
        }
        o.extend_from_slice(
            format!("trailer\n<< /Size {size} /Root 1 0 R /Info 4 0 R >>\nstartxref\n{x}\n%%EOF\n")
                .as_bytes(),
        );
        o
    }

    fn num_after(t: &str, key: &str) -> usize {
        let i = t.find(key).unwrap() + key.len();
        t[i..]
            .trim_start()
            .split(|c: char| !c.is_ascii_digit())
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    #[test]
    fn linearized_file_keeps_its_pages_and_states_true_offsets() {
        for n in [1usize, 2, 5] {
            let src = sample(n);
            let l = linearize(&src).unwrap();
            let head = String::from_utf8_lossy(&l[..300]).into_owned();
            assert!(head.contains("/Linearized 1"), "{head}");
            assert_eq!(num_after(&head, "/L "), l.len(), "n={n}");
            assert_eq!(num_after(&head, "/N "), n);
            // the hint stream sits where /H says
            let h = head.split("/H [").nth(1).unwrap();
            let mut it = h.split_whitespace();
            let off: usize = it.next().unwrap().parse().unwrap();
            let len: usize = it.next().unwrap().parse().unwrap();
            let at = String::from_utf8_lossy(&l[off..off + 20]).into_owned();
            assert!(at.contains(" 0 obj"), "{at}");
            assert_eq!(l[off + len - 1], b'\n');
            // the first page object follows right after it, and /E is the end of its section
            let e = num_after(&head, "/E ");
            assert!(e > off + len && e <= l.len());
            let o = num_after(&head, "/O ");
            let first = String::from_utf8_lossy(&l[off + len..off + len + 12]).into_owned();
            assert!(first.starts_with(&format!("{o} 0 obj")), "{first}");
            let doc = PdfDoc::open(l.clone()).unwrap();
            assert_eq!(doc.page_count(), n);
            // linearizing is not done twice
            assert!(linearize(&l).is_err() || PdfDoc::open(linearize(&l).unwrap()).is_ok());
        }
    }

    #[test]
    fn encrypted_file_is_refused() {
        let enc =
            super::super::encrypt::encrypt(&sample(2), "a", "b", 0, &mut |b: &mut [u8]| b.fill(3))
                .unwrap();
        assert!(linearize(&enc).is_err());
    }

    #[test]
    fn a_linearized_file_can_be_encrypted_as_it_is_written() {
        use super::super::encrypt::Protect;
        let p = Protect {
            user: "open",
            owner: "boss",
            allowed: 0,
        };
        let mut n = 0u8;
        let mut rng = |b: &mut [u8]| {
            for x in b.iter_mut() {
                n = n.wrapping_add(7);
                *x = n;
            }
        };
        for pages in [1usize, 3, 5] {
            let l = linearize_with(&sample(pages), Some(&p), &mut rng).unwrap();
            let t = String::from_utf8_lossy(&l[..300]).into_owned();
            assert!(t.contains("/Linearized 1"));
            let all = String::from_utf8_lossy(&l).into_owned();
            assert!(all.contains("/Encrypt ") && all.contains("/CFM /AESV3"));
            assert!(!all.contains("0 0 50 50 re f"), "content is readable");
            assert!(PdfDoc::open(l.clone()).is_err());
            assert!(PdfDoc::open_with_password(l.clone(), b"wrong").is_err());
            let d = PdfDoc::open_with_password(l.clone(), b"open").unwrap();
            assert_eq!(d.page_count(), pages);
            // the declared length is the real length and the hint stream offset points at an object
            let len = num_after(&t, "/L ");
            assert_eq!(len, l.len());
            let h = t.split("/H [").nth(1).unwrap();
            let off: usize = h.split_whitespace().next().unwrap().parse().unwrap();
            assert!(String::from_utf8_lossy(&l[off..off + 20]).contains(" 0 obj"));
            // every entry of the first-page cross-reference table points at its object
            let x = all.find("\nxref\n").unwrap() + 6;
            let mut lines = all[x..].lines();
            let head: Vec<usize> = lines
                .next()
                .unwrap()
                .split(' ')
                .map(|v| v.parse().unwrap())
                .collect();
            for k in 0..head[1] {
                let e = lines.next().unwrap();
                let o: usize = e[..10].parse().unwrap();
                let want = format!("{} 0 obj", head[0] + k);
                assert!(
                    String::from_utf8_lossy(&l[o..o + want.len()]) == want,
                    "entry {k} of {pages} pages points at {:?}, not {want}",
                    String::from_utf8_lossy(&l[o..o + want.len()])
                );
            }
        }
    }
}

#[cfg(test)]
mod dump {
    /// Write sample files for hand checks with qpdf and poppler (only with LGR_DUMP_DIR set).
    #[test]
    fn dump_samples() {
        let Ok(dir) = std::env::var("LGR_DUMP_DIR") else {
            return;
        };
        for n in [1usize, 2, 5] {
            let l = super::linearize(&super::tests::sample(n)).unwrap();
            std::fs::write(format!("{dir}/lin{n}.pdf"), l).unwrap();
        }
    }
}
