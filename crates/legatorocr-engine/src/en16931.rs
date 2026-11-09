//! Business-rule checks on an electronic invoice in the EN 16931 semantic model (calculation
//! rules BR-CO-*, decimals BR-DEC, mandatory fields BR-*), reported by rule number.
//!
//! The rules are applied to a [`Semantic`] value, so they do not depend on a syntax. The UBL
//! reader [`from_ubl`] fills only the fields whose element names are known to this program;
//! a rule is applied only to fields the source could read (`Semantic::read`), so that a field
//! the reader does not know is never reported as missing. This is not a full Schematron
//! validation: code lists, national rules (CIUS-RO) and many conditional rules are not covered.

use std::collections::{BTreeMap, BTreeSet};

/// Exact decimal number: `m / 10^s`. No binary floating point is involved anywhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dec {
    m: i128,
    s: u32,
}

impl Dec {
    /// Plain decimal text: optional sign, digits, optional `.digits`. At most 18 digits in all.
    pub fn parse(t: &str) -> Option<Dec> {
        let t = t.trim();
        let (neg, body) = match t.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, t.strip_prefix('+').unwrap_or(t)),
        };
        let (int, frac) = body.split_once('.').unwrap_or((body, ""));
        if (int.is_empty() && frac.is_empty())
            || !int.bytes().chain(frac.bytes()).all(|b| b.is_ascii_digit())
            || int.len() + frac.len() > 18
        {
            return None;
        }
        let m: i128 = format!("{int}{frac}").parse().ok()?;
        Some(Dec {
            m: if neg { -m } else { m },
            s: frac.len() as u32,
        })
    }

    fn lift(self, s: u32) -> i128 {
        self.m * 10i128.pow(s - self.s)
    }

    pub fn plus(self, o: Dec) -> Dec {
        let s = self.s.max(o.s);
        Dec {
            m: self.lift(s) + o.lift(s),
            s,
        }
    }

    pub fn negated(self) -> Dec {
        Dec {
            m: -self.m,
            s: self.s,
        }
    }

    pub fn minus(self, o: Dec) -> Dec {
        self.plus(o.negated())
    }

    pub fn zero() -> Dec {
        Dec { m: 0, s: 0 }
    }

    pub fn eq_value(self, o: Dec) -> bool {
        let s = self.s.max(o.s);
        self.lift(s) == o.lift(s)
    }

    fn abs_le(self, limit: Dec) -> bool {
        let s = self.s.max(limit.s);
        self.lift(s).abs() <= limit.lift(s)
    }

    /// Number of decimals the value really needs (trailing zeros do not count).
    pub fn decimals(self) -> u32 {
        let (mut m, mut s) = (self.m, self.s);
        while s > 0 && m % 10 == 0 {
            m /= 10;
            s -= 1;
        }
        s
    }

    /// `self * rate / 100`, exact.
    pub fn percent(self, rate: Dec) -> Dec {
        Dec {
            m: self.m * rate.m,
            s: self.s + rate.s + 2,
        }
    }
}

impl std::fmt::Display for Dec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // at least two decimals, more only when the value needs them
        let want = self.decimals().max(2);
        let v = self.lift(want.max(self.s));
        let s = want.max(self.s);
        let (sign, a) = (if v < 0 { "-" } else { "" }, v.abs());
        let p = 10i128.pow(s);
        write!(f, "{sign}{}.{:0w$}", a / p, a % p, w = s as usize)
    }
}

/// An allowance or a charge (document or line level).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowCharge {
    pub amount: Option<String>,
    pub reason_text: Option<String>,
    pub reason_code: Option<String>,
}

/// A period: start and end dates, either may be empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Period {
    pub start: Option<String>,
    pub end: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VatCategory {
    pub taxable: Option<String>,
    pub rate: Option<String>,
    pub amount: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SemLine {
    /// BT-126, BT-129, BT-130, BT-131, BT-153, BT-146 in this order.
    pub id: Option<String>,
    pub quantity: Option<String>,
    pub unit: Option<String>,
    pub net_amount: Option<String>,
    pub item_name: Option<String>,
    pub net_price: Option<String>,
    pub allowances: Vec<AllowCharge>,
    pub period: Option<Period>,
}

/// The invoice as business terms. `fields` is keyed by BT number ("BT-1", ...), text as written.
#[derive(Clone, Debug, Default)]
pub struct Semantic {
    pub fields: BTreeMap<&'static str, String>,
    pub lines: Vec<SemLine>,
    pub allowances: Vec<AllowCharge>,
    pub charges: Vec<AllowCharge>,
    pub vat: Vec<VatCategory>,
    pub period: Option<Period>,
    /// Which parts the source could read: BT numbers, and the group keys `lines`, `line.BT-126`,
    /// `line.BT-131` (and the other line terms), `allowances`, `charges`, `line.allowances`, `vat`,
    /// `period`, `line.period`. A rule about something outside this set is not applied.
    pub read: BTreeSet<&'static str>,
}

impl Semantic {
    /// A source that can read everything (for tests and for callers that fill all terms).
    pub fn all_read() -> Semantic {
        let mut s = Semantic::default();
        for k in ALL_KEYS {
            s.read.insert(k);
        }
        s
    }
}

const ALL_KEYS: &[&str] = &[
    "BT-1",
    "BT-2",
    "BT-3",
    "BT-5",
    "BT-24",
    "BT-44",
    "BT-40",
    "BT-55",
    "BT-106",
    "BT-107",
    "BT-108",
    "BT-109",
    "BT-110",
    "BT-112",
    "BT-113",
    "BT-114",
    "BT-115",
    "BT-31",
    "BT-48",
    "BT-63",
    "BT-7",
    "BT-8",
    "lines",
    "line.BT-126",
    "line.BT-129",
    "line.BT-130",
    "line.BT-131",
    "line.BT-153",
    "line.BT-146",
    "allowances",
    "line.allowances",
    "vat",
    "period",
    "line.period",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    /// Rule identifier, e.g. `BR-CO-16`; `BR-DEC` when the number of the decimals rule for the
    /// field is not known to this program; `NUMBER` for text that is not a decimal.
    pub rule: String,
    pub fields: Vec<String>,
    pub found: String,
    pub expected: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [{}]: found {}, expected {}",
            self.rule,
            self.fields.join(", "),
            self.found,
            self.expected
        )
    }
}

struct Ctx<'a> {
    s: &'a Semantic,
    out: Vec<Violation>,
}

impl Ctx<'_> {
    fn bad(&mut self, rule: &str, fields: &[&str], found: String, expected: String) {
        self.out.push(Violation {
            rule: rule.to_string(),
            fields: fields.iter().map(|x| x.to_string()).collect(),
            found,
            expected,
        });
    }

    fn read(&self, k: &str) -> bool {
        self.s.read.contains(k)
    }

    fn text(&self, k: &str) -> Option<&str> {
        self.s
            .fields
            .get(k)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }

    /// Amount of a BT; an unreadable number is reported once and then treated as absent.
    fn amount(&mut self, k: &'static str) -> Option<Dec> {
        let t = self.text(k)?.to_string();
        self.dec(&t, k)
    }

    fn dec(&mut self, t: &str, field: &str) -> Option<Dec> {
        let d = Dec::parse(t);
        if d.is_none() {
            self.bad(
                "NUMBER",
                &[field],
                t.to_string(),
                "a decimal number".to_string(),
            );
        }
        d
    }

    fn equal(&mut self, rule: &str, fields: &[&str], found: Dec, expected: Dec) {
        if !found.eq_value(expected) {
            self.bad(rule, fields, found.to_string(), expected.to_string());
        }
    }

    fn mandatory(&mut self, rule: &str, bt: &'static str) {
        if self.read(bt) && self.text(bt).is_none() {
            self.bad(rule, &[bt], "absent".into(), "a value".into());
        }
    }
}

fn present(o: &Option<String>) -> bool {
    o.as_deref().is_some_and(|v| !v.trim().is_empty())
}

/// Apply every rule this program knows to `s`; each violated rule is reported on its own.
pub fn check(s: &Semantic) -> Vec<Violation> {
    let mut c = Ctx { s, out: Vec::new() };
    for (rule, bt) in [
        ("BR-01", "BT-24"),
        ("BR-02", "BT-1"),
        ("BR-03", "BT-2"),
        ("BR-04", "BT-3"),
        ("BR-05", "BT-5"),
        ("BR-07", "BT-44"),
        ("BR-09", "BT-40"),
        ("BR-11", "BT-55"),
        ("BR-12", "BT-106"),
        ("BR-13", "BT-109"),
        ("BR-15", "BT-115"),
    ] {
        c.mandatory(rule, bt);
    }
    if c.read("lines") && s.lines.is_empty() {
        c.bad("BR-16", &["lines"], "0 lines".into(), "at least 1".into());
    }
    for l in &s.lines {
        for (rule, key, v) in [
            ("BR-21", "line.BT-126", &l.id),
            ("BR-22", "line.BT-129", &l.quantity),
            ("BR-23", "line.BT-130", &l.unit),
            ("BR-24", "line.BT-131", &l.net_amount),
            ("BR-25", "line.BT-153", &l.item_name),
            ("BR-26", "line.BT-146", &l.net_price),
        ] {
            if c.read(key) && !present(v) {
                c.bad(rule, &[key], "absent".into(), "a value".into());
            }
        }
    }

    // amounts
    let line_total = c.amount("BT-106");
    let allow_total = c.amount("BT-107");
    let charge_total = c.amount("BT-108");
    let net = c.amount("BT-109");
    let vat_total = c.amount("BT-110");
    let gross = c.amount("BT-112");
    let prepaid = c.amount("BT-113");
    let rounding = c.amount("BT-114");
    let payable = c.amount("BT-115");

    // BR-CO-10: sum of line net amounts
    if c.read("lines") && c.read("line.BT-131") {
        if let Some(t) = line_total {
            let mut sum = Dec::zero();
            let mut ok = true;
            for l in &s.lines {
                if let Some(a) = l.net_amount.as_deref().filter(|x| !x.trim().is_empty()) {
                    match c.dec(a, "BT-131") {
                        Some(d) => sum = sum.plus(d),
                        None => ok = false,
                    }
                }
            }
            if ok {
                c.equal("BR-CO-10", &["BT-106", "BT-131"], t, sum);
            }
        }
    }
    // BR-CO-11 / BR-CO-12: declared totals of document allowances / charges
    for (rule, bt, key, declared, list) in [
        (
            "BR-CO-11",
            "BT-107",
            "allowances",
            allow_total,
            &s.allowances,
        ),
        ("BR-CO-12", "BT-108", "charges", charge_total, &s.charges),
    ] {
        if let (true, Some(t)) = (c.read(key), declared) {
            let mut sum = Dec::zero();
            for a in list {
                if let Some(x) = a.amount.as_deref().and_then(Dec::parse) {
                    sum = sum.plus(x);
                }
            }
            c.equal(rule, &[bt, key], t, sum);
        }
    }
    // BR-CO-13: net total = lines - allowances + charges
    if let (Some(n), Some(l)) = (net, line_total) {
        if c.read("BT-107") && c.read("BT-108") {
            let expected = l
                .minus(allow_total.unwrap_or(Dec::zero()))
                .plus(charge_total.unwrap_or(Dec::zero()));
            c.equal(
                "BR-CO-13",
                &["BT-109", "BT-106", "BT-107", "BT-108"],
                n,
                expected,
            );
        }
    }
    // BR-CO-14: total VAT = sum of the category amounts, when both are given
    if c.read("vat") && !s.vat.is_empty() {
        if let Some(t) = vat_total {
            let mut sum = Dec::zero();
            for v in &s.vat {
                if let Some(x) = v.amount.as_deref().and_then(Dec::parse) {
                    sum = sum.plus(x);
                }
            }
            c.equal("BR-CO-14", &["BT-110", "BT-117"], t, sum);
        }
    }
    // BR-CO-15: gross = net + VAT, when VAT is given
    if let (Some(g), Some(n), Some(v)) = (gross, net, vat_total) {
        c.equal("BR-CO-15", &["BT-112", "BT-109", "BT-110"], g, n.plus(v));
    }
    // BR-CO-16: payable = gross - paid + rounding
    if let (Some(p), Some(g)) = (payable, gross) {
        if c.read("BT-113") && c.read("BT-114") {
            let expected = g
                .minus(prepaid.unwrap_or(Dec::zero()))
                .plus(rounding.unwrap_or(Dec::zero()));
            c.equal(
                "BR-CO-16",
                &["BT-115", "BT-112", "BT-113", "BT-114"],
                p,
                expected,
            );
        }
    }
    // BR-CO-17: VAT per category = base x rate / 100, rounded to two decimals. The rounding
    // direction is not settled, so the amount is accepted when it lies within half a hundredth
    // of the exact product, which every rounding to two decimals satisfies.
    if c.read("vat") {
        for v in &s.vat {
            if !(present(&v.taxable) && present(&v.rate) && present(&v.amount)) {
                continue;
            }
            let (Some(b), Some(r), Some(a)) = (
                c.dec(v.taxable.as_deref().unwrap_or(""), "BT-116"),
                c.dec(v.rate.as_deref().unwrap_or(""), "BT-119"),
                c.dec(v.amount.as_deref().unwrap_or(""), "BT-117"),
            ) else {
                continue;
            };
            let exact = b.percent(r);
            if !a.minus(exact).abs_le(Dec { m: 5, s: 3 }) {
                c.bad(
                    "BR-CO-17",
                    &["BT-117", "BT-116", "BT-119"],
                    a.to_string(),
                    format!("{} (exact {exact})", Dec { m: 0, s: 0 }.plus(round2(exact))),
                );
            }
        }
        for v in &s.vat {
            if !present(&v.taxable) {
                c.bad("BR-45", &["BT-116"], "absent".into(), "a value".into());
            }
        }
    }
    // BR-CO-19 / BR-CO-20: a period needs a start, an end or both
    if c.read("period") {
        if let Some(p) = &s.period {
            if !present(&p.start) && !present(&p.end) {
                c.bad(
                    "BR-CO-19",
                    &["BT-73", "BT-74"],
                    "none".into(),
                    "a start, an end or both".into(),
                );
            }
        }
    }
    if c.read("line.period") {
        for l in &s.lines {
            if let Some(p) = &l.period {
                if !present(&p.start) && !present(&p.end) {
                    c.bad(
                        "BR-CO-20",
                        &["BT-134", "BT-135"],
                        "none".into(),
                        "a start, an end or both".into(),
                    );
                }
            }
        }
    }
    // allowances: amount (BR-31 / BR-41) and a reason in text or code (BR-CO-21 / BR-CO-23)
    if c.read("allowances") {
        for a in &s.allowances {
            if !present(&a.amount) {
                c.bad("BR-31", &["BT-92"], "absent".into(), "a value".into());
            }
            if !present(&a.reason_text) && !present(&a.reason_code) {
                c.bad(
                    "BR-CO-21",
                    &["BT-97", "BT-98"],
                    "none".into(),
                    "a reason text, a reason code or both".into(),
                );
            }
        }
    }
    if c.read("line.allowances") {
        for a in s.lines.iter().flat_map(|l| &l.allowances) {
            if !present(&a.amount) {
                c.bad("BR-41", &["BT-136"], "absent".into(), "a value".into());
            }
            if !present(&a.reason_text) && !present(&a.reason_code) {
                c.bad(
                    "BR-CO-23",
                    &["BT-139", "BT-140"],
                    "none".into(),
                    "a reason text, a reason code or both".into(),
                );
            }
        }
    }
    // BR-CO-03: tax point date and tax point code exclude each other
    if c.text("BT-7").is_some() && c.text("BT-8").is_some() {
        c.bad(
            "BR-CO-03",
            &["BT-7", "BT-8"],
            "both given".into(),
            "only one".into(),
        );
    }
    // BR-CO-09: VAT identifiers start with a country prefix
    for bt in ["BT-31", "BT-48", "BT-63"] {
        if let Some(id) = c.text(bt) {
            let p: Vec<char> = id.chars().take(2).collect();
            if !(p.len() == 2 && p.iter().all(|ch| ch.is_ascii_alphabetic())) {
                let found = id.to_string();
                c.bad(
                    "BR-CO-09",
                    &[bt],
                    found,
                    "two letters of a country prefix first".into(),
                );
            }
        }
    }
    // BR-DEC: at most two decimals in money amounts
    for bt in [
        "BT-106", "BT-107", "BT-108", "BT-109", "BT-110", "BT-112", "BT-113", "BT-114", "BT-115",
    ] {
        if let Some(t) = c.text(bt).map(str::to_string) {
            if Dec::parse(&t).is_some_and(|d| d.decimals() > 2) {
                c.bad("BR-DEC", &[bt], t, "at most 2 decimals".into());
            }
        }
    }
    let mut decs: Vec<(&str, String)> = Vec::new();
    for l in &s.lines {
        if let Some(a) = &l.net_amount {
            decs.push(("BT-131", a.clone()));
        }
    }
    for a in &s.allowances {
        if let Some(x) = &a.amount {
            decs.push(("BT-92", x.clone()));
        }
    }
    for v in &s.vat {
        for (bt, x) in [("BT-116", &v.taxable), ("BT-117", &v.amount)] {
            if let Some(x) = x {
                decs.push((bt, x.clone()));
            }
        }
    }
    for (bt, t) in decs {
        if Dec::parse(&t).is_some_and(|d| d.decimals() > 2) {
            c.bad("BR-DEC", &[bt], t, "at most 2 decimals".into());
        }
    }
    c.out
}

/// Round half away from zero to two decimals (used only to show an example expected value).
fn round2(d: Dec) -> Dec {
    let target = 2;
    if d.s <= target {
        return d;
    }
    let div = 10i128.pow(d.s - target);
    let half = div / 2;
    let q = if d.m >= 0 {
        (d.m + half) / div
    } else {
        -((-d.m + half) / div)
    };
    Dec { m: q, s: target }
}

/// Read what this program knows how to read from a UBL Invoice or CreditNote. Element names
/// used: those of `cac:LegalMonetaryTotal`, `cac:TaxTotal/cbc:TaxAmount`, the identifiers and
/// dates at the top of the document, and for lines `cbc:ID` and `cbc:LineExtensionAmount`.
pub fn from_ubl(xml: &str) -> anyhow::Result<Semantic> {
    let inv = crate::efactura::parse(xml)?;
    let els = crate::efactura::flatten(xml);
    let root = els
        .first()
        .map(|e| e.0.split('/').next().unwrap_or("").to_string())
        .unwrap_or_default();
    let get = |p: &str| -> Option<String> {
        let full = format!("{root}/{p}");
        els.iter().find(|e| e.0 == full).map(|e| e.1.clone())
    };
    let mut s = Semantic::default();
    let mut put = |bt: &'static str, v: Option<String>| {
        s.read.insert(bt);
        if let Some(v) = v {
            s.fields.insert(bt, v);
        }
    };
    put("BT-24", get("CustomizationID"));
    put("BT-1", get("ID"));
    put("BT-2", get("IssueDate"));
    put("BT-5", get("DocumentCurrencyCode"));
    put("BT-106", get("LegalMonetaryTotal/LineExtensionAmount"));
    put("BT-109", get("LegalMonetaryTotal/TaxExclusiveAmount"));
    put("BT-112", get("LegalMonetaryTotal/TaxInclusiveAmount"));
    put("BT-107", get("LegalMonetaryTotal/AllowanceTotalAmount"));
    put("BT-108", get("LegalMonetaryTotal/ChargeTotalAmount"));
    put("BT-113", get("LegalMonetaryTotal/PrepaidAmount"));
    put("BT-114", get("LegalMonetaryTotal/PayableRoundingAmount"));
    put("BT-115", get("LegalMonetaryTotal/PayableAmount"));
    put("BT-110", get("TaxTotal/TaxAmount"));
    let name = inv.customer.name.clone();
    put("BT-44", (!name.is_empty()).then_some(name));
    let (sv, cv) = (inv.supplier.tax_id.clone(), inv.customer.tax_id.clone());
    put("BT-31", (!sv.is_empty()).then_some(sv));
    put("BT-48", (!cv.is_empty()).then_some(cv));
    let line_tag = if root == "Invoice" {
        "InvoiceLine"
    } else {
        "CreditNoteLine"
    };
    // lines: each line starts at its own `ID` element
    let mut lines: Vec<SemLine> = Vec::new();
    let id_path = format!("{root}/{line_tag}/ID");
    let amt_path = format!("{root}/{line_tag}/LineExtensionAmount");
    for (p, v) in &els {
        if *p == id_path {
            lines.push(SemLine {
                id: Some(v.clone()),
                ..SemLine::default()
            });
        } else if *p == amt_path {
            if let Some(l) = lines.last_mut() {
                l.net_amount = Some(v.clone());
            }
        }
    }
    s.lines = lines;
    for k in ["lines", "line.BT-126", "line.BT-131"] {
        s.read.insert(k);
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete, correct invoice: one line of 100.00, VAT 19% = 19.00.
    fn good() -> Semantic {
        let mut s = Semantic::all_read();
        for (k, v) in [
            ("BT-24", "urn:cen.eu:en16931:2017"),
            ("BT-1", "F1"),
            ("BT-2", "2026-11-09"),
            ("BT-3", "380"),
            ("BT-5", "EUR"),
            ("BT-44", "Buyer"),
            ("BT-40", "RO"),
            ("BT-55", "RO"),
            ("BT-106", "100.00"),
            ("BT-109", "100.00"),
            ("BT-110", "19.00"),
            ("BT-112", "119.00"),
            ("BT-113", "0.00"),
            ("BT-115", "119.00"),
        ] {
            s.fields.insert(k, v.to_string());
        }
        s.lines.push(SemLine {
            id: Some("1".into()),
            quantity: Some("1".into()),
            unit: Some("C62".into()),
            net_amount: Some("100.00".into()),
            item_name: Some("Thing".into()),
            net_price: Some("100.00".into()),
            ..SemLine::default()
        });
        s.vat.push(VatCategory {
            taxable: Some("100.00".into()),
            rate: Some("19".into()),
            amount: Some("19.00".into()),
        });
        s
    }

    fn rules(s: &Semantic) -> Vec<String> {
        check(s).into_iter().map(|v| v.rule).collect()
    }

    #[test]
    fn decimals_are_exact() {
        let a = Dec::parse("0.1").unwrap();
        let b = Dec::parse("0.2").unwrap();
        assert!(a.plus(b).eq_value(Dec::parse("0.30").unwrap()));
        assert_eq!(Dec::parse("12.345").unwrap().decimals(), 3);
        assert_eq!(Dec::parse("12.340").unwrap().decimals(), 2);
        assert_eq!(Dec::parse("-5").unwrap().to_string(), "-5.00");
        assert!(
            Dec::parse("1e3").is_none() && Dec::parse("").is_none() && Dec::parse("1,5").is_none()
        );
    }

    #[test]
    fn a_correct_invoice_has_no_violation() {
        assert!(rules(&good()).is_empty(), "{:?}", check(&good()));
    }

    #[test]
    fn wrong_payable_is_br_co_16_with_the_expected_value() {
        let mut s = good();
        s.fields.insert("BT-115", "120.00".into());
        let v = check(&s);
        assert_eq!(v.len(), 1, "{v:?}");
        assert_eq!(
            (v[0].rule.as_str(), v[0].expected.as_str()),
            ("BR-CO-16", "119.00")
        );
        assert_eq!(v[0].found, "120.00");
    }

    #[test]
    fn wrong_line_total_is_br_co_10() {
        let mut s = good();
        s.lines[0].net_amount = Some("10.00".into());
        s.lines.push(SemLine {
            net_amount: Some("5.50".into()),
            ..s.lines[0].clone()
        });
        s.fields.insert("BT-106", "15.00".into());
        let v: Vec<_> = check(&s)
            .into_iter()
            .filter(|v| v.rule == "BR-CO-10")
            .collect();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].expected, "15.50");
    }

    #[test]
    fn vat_per_category_is_checked_within_rounding() {
        let mut s = good();
        s.vat[0].amount = Some("18.99".into());
        s.fields.insert("BT-110", "18.99".into());
        s.fields.insert("BT-112", "118.99".into());
        s.fields.insert("BT-115", "118.99".into());
        assert_eq!(rules(&s), vec!["BR-CO-17"]);
        // a base whose exact product has a third decimal accepts both roundings
        for amt in ["0.87", "0.88"] {
            let mut t = good();
            t.vat[0] = VatCategory {
                taxable: Some("10.00".into()),
                rate: Some("8.75".into()),
                amount: Some(amt.into()),
            };
            t.fields.insert("BT-110", amt.into());
            let sum = Dec::parse("100.00").unwrap().plus(Dec::parse(amt).unwrap());
            t.fields.insert("BT-112", sum.to_string());
            t.fields.insert("BT-115", sum.to_string());
            assert!(rules(&t).is_empty(), "{amt}: {:?}", check(&t));
        }
    }

    #[test]
    fn three_decimals_violate_the_decimals_rule() {
        let mut s = good();
        s.fields.insert("BT-113", "12.345".into());
        assert!(rules(&s).contains(&"BR-DEC".to_string()));
    }

    #[test]
    fn document_allowance_and_charge_in_the_net_total() {
        let mut s = good();
        s.fields.insert("BT-107", "10.00".into());
        s.fields.insert("BT-108", "5.00".into());
        s.fields.insert("BT-109", "95.00".into());
        s.fields.insert("BT-110", "18.05".into());
        s.fields.insert("BT-112", "113.05".into());
        s.fields.insert("BT-115", "113.05".into());
        s.vat[0] = VatCategory {
            taxable: Some("95.00".into()),
            rate: Some("19".into()),
            amount: Some("18.05".into()),
        };
        s.allowances.push(AllowCharge {
            amount: Some("10.00".into()),
            reason_text: Some("loyalty".into()),
            reason_code: None,
        });
        assert!(rules(&s).is_empty(), "{:?}", check(&s));
        s.fields.insert("BT-109", "105.00".into());
        let r = rules(&s);
        assert!(r.contains(&"BR-CO-13".to_string()), "{r:?}");
    }

    #[test]
    fn missing_number_date_and_lines() {
        let mut s = good();
        s.fields.remove("BT-1");
        s.fields.remove("BT-2");
        s.lines.clear();
        let r = rules(&s);
        for want in ["BR-02", "BR-03", "BR-16"] {
            assert!(r.contains(&want.to_string()), "{want} in {r:?}");
        }
    }

    #[test]
    fn allowance_without_amount_or_reason() {
        let mut s = good();
        s.allowances.push(AllowCharge::default());
        let r = rules(&s);
        assert!(
            r.contains(&"BR-31".to_string()) && r.contains(&"BR-CO-21".to_string()),
            "{r:?}"
        );
        let mut t = good();
        t.lines[0].allowances.push(AllowCharge::default());
        let r = rules(&t);
        assert!(
            r.contains(&"BR-41".to_string()) && r.contains(&"BR-CO-23".to_string()),
            "{r:?}"
        );
    }

    #[test]
    fn small_rules_period_tax_point_and_vat_prefix() {
        let mut s = good();
        s.period = Some(Period::default());
        s.fields.insert("BT-7", "2026-11-01".into());
        s.fields.insert("BT-8", "5".into());
        s.fields.insert("BT-31", "1234567".into());
        s.fields.insert("BT-48", "EL123456789".into());
        let r = rules(&s);
        for want in ["BR-CO-19", "BR-CO-03", "BR-CO-09"] {
            assert!(r.contains(&want.to_string()), "{want} in {r:?}");
        }
        assert_eq!(r.iter().filter(|x| *x == "BR-CO-09").count(), 1);
    }

    #[test]
    fn a_field_the_reader_does_not_know_is_not_reported_missing() {
        let xml = r#"<Invoice xmlns:cbc="a" xmlns:cac="b">
<cbc:CustomizationID>x</cbc:CustomizationID><cbc:ID>F1</cbc:ID><cbc:IssueDate>2026-11-09</cbc:IssueDate>
<cbc:DocumentCurrencyCode>EUR</cbc:DocumentCurrencyCode>
<cac:AccountingCustomerParty><cac:Party><cac:PartyLegalEntity><cbc:RegistrationName>B</cbc:RegistrationName></cac:PartyLegalEntity></cac:Party></cac:AccountingCustomerParty>
<cac:LegalMonetaryTotal><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount><cbc:TaxExclusiveAmount>100.00</cbc:TaxExclusiveAmount>
<cbc:TaxInclusiveAmount>119.00</cbc:TaxInclusiveAmount><cbc:PayableAmount>120.00</cbc:PayableAmount></cac:LegalMonetaryTotal>
<cac:TaxTotal><cbc:TaxAmount>19.00</cbc:TaxAmount></cac:TaxTotal>
<cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount></cac:InvoiceLine>
</Invoice>"#;
        let s = from_ubl(xml).unwrap();
        let r = rules(&s);
        // type code, countries, quantity ... are unknown to the reader and are not reported;
        // the payable amount is wrong
        assert_eq!(r, vec!["BR-CO-16"], "{:?}", check(&s));
        let no_id = xml.replace("<cbc:ID>F1</cbc:ID>", "");
        assert!(rules(&from_ubl(&no_id).unwrap()).contains(&"BR-02".to_string()));
        let no_lines = xml.replace(
            "<cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:LineExtensionAmount>100.00</cbc:LineExtensionAmount></cac:InvoiceLine>",
            "",
        );
        assert!(rules(&from_ubl(&no_lines).unwrap()).contains(&"BR-16".to_string()));
    }
}
