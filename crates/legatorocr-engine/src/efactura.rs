//! e-Factura (RO_CIUS, UBL 2.1) invoice reader: key fields and arithmetic checks (roadmap 6.10).
//!
//! Reads the XML file (or the first non-signature `.xml` inside the ZIP that ANAF returns) and
//! reports what an accountant looks at first. It does not validate against the CIUS-RO schematron
//! and does not check signatures; it only reports fields and whether the amounts add up and the
//! Romanian tax identifiers pass their check digit.

use anyhow::{bail, Result};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Party {
    pub name: String,
    /// VAT / tax id as written, e.g. `RO1000009`.
    pub tax_id: String,
    pub legal_id: String,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Invoice {
    pub kind: String,
    pub id: String,
    pub issue_date: String,
    pub due_date: String,
    pub currency: String,
    pub customization: String,
    pub supplier: Party,
    pub customer: Party,
    pub iban: String,
    pub lines: usize,
    pub line_sum: Option<f64>,
    pub net: Option<f64>,
    pub tax: Option<f64>,
    pub gross: Option<f64>,
    pub prepaid: Option<f64>,
    pub payable: Option<f64>,
}

/// Flatten XML into `(path, text)` for every element with text; the path is local names
/// joined by `/` (namespace prefixes dropped).
fn elements(xml: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut i = 0;
    let b = xml.as_bytes();
    while i < b.len() {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        let Some(end) = xml[i..].find('>') else { break };
        let tag = &xml[i + 1..i + end];
        let next = i + end + 1;
        if tag.starts_with('?') || tag.starts_with('!') {
            // comments may contain '>'
            if let Some(rest) = tag.strip_prefix("!--") {
                let _ = rest;
                if let Some(c) = xml[i..].find("-->") {
                    i += c + 3;
                    continue;
                }
            }
            i = next;
            continue;
        }
        if let Some(close) = tag.strip_prefix('/') {
            let _ = close;
            stack.pop();
            i = next;
            continue;
        }
        let selfclose = tag.ends_with('/');
        let name = tag
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or("");
        let local = name.rsplit(':').next().unwrap_or(name);
        if selfclose {
            i = next;
            continue;
        }
        stack.push(local);
        // text up to the next tag
        let text_end = xml[next..].find('<').map(|p| next + p).unwrap_or(xml.len());
        let text = xml[next..text_end].trim();
        if !text.is_empty() {
            out.push((stack.join("/"), unescape(text)));
        }
        i = next;
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Romanian CUI check digit: weights 7 5 3 2 1 7 5 3 2 over the digits before the last,
/// (sum * 10) mod 11, with 10 mapping to 0. Accepts an `RO` prefix.
pub fn cui_valid(id: &str) -> bool {
    let d: String = id
        .trim()
        .trim_start_matches("RO")
        .trim_start_matches("ro")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if d.len() < 2 || d.len() > 10 || !d.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    let digits: Vec<u32> = d.bytes().map(|b| (b - b'0') as u32).collect();
    let (body, check) = digits.split_at(digits.len() - 1);
    const W: [u32; 9] = [7, 5, 3, 2, 1, 7, 5, 3, 2];
    let w = &W[9 - body.len()..];
    let s: u32 = body.iter().zip(w).map(|(a, b)| a * b).sum();
    let mut c = (s * 10) % 11;
    if c == 10 {
        c = 0;
    }
    c == check[0]
}

fn num(s: &str) -> Option<f64> {
    s.trim().parse().ok()
}

pub fn parse(xml: &str) -> Result<Invoice> {
    let els = elements(xml);
    let Some(root) = els
        .first()
        .map(|e| e.0.split('/').next().unwrap_or("").to_string())
    else {
        bail!("empty XML");
    };
    if root != "Invoice" && root != "CreditNote" {
        bail!("not a UBL Invoice or CreditNote (root element is {root})");
    }
    let get = |p: &str| -> String {
        let full = format!("{root}/{p}");
        els.iter()
            .find(|e| e.0 == full)
            .map(|e| e.1.clone())
            .unwrap_or_default()
    };
    let party = |side: &str| Party {
        name: {
            let n = get(&format!("{side}/Party/PartyLegalEntity/RegistrationName"));
            if n.is_empty() {
                get(&format!("{side}/Party/PartyName/Name"))
            } else {
                n
            }
        },
        tax_id: get(&format!("{side}/Party/PartyTaxScheme/CompanyID")),
        legal_id: get(&format!("{side}/Party/PartyLegalEntity/CompanyID")),
    };
    let line_tag = if root == "Invoice" {
        "InvoiceLine"
    } else {
        "CreditNoteLine"
    };
    let line_prefix = format!("{root}/{line_tag}/");
    let lines = els
        .iter()
        .filter(|e| e.0 == format!("{root}/{line_tag}/ID"))
        .count();
    let line_sum: f64 = els
        .iter()
        .filter(|e| e.0 == format!("{line_prefix}LineExtensionAmount"))
        .filter_map(|e| num(&e.1))
        .sum();
    Ok(Invoice {
        kind: root.clone(),
        id: get("ID"),
        issue_date: get("IssueDate"),
        due_date: get("DueDate"),
        currency: get("DocumentCurrencyCode"),
        customization: get("CustomizationID"),
        supplier: party("AccountingSupplierParty"),
        customer: party("AccountingCustomerParty"),
        iban: get("PaymentMeans/PayeeFinancialAccount/ID"),
        lines,
        line_sum: if lines > 0 { Some(line_sum) } else { None },
        net: num(&get("LegalMonetaryTotal/TaxExclusiveAmount")),
        tax: num(&get("TaxTotal/TaxAmount")),
        gross: num(&get("LegalMonetaryTotal/TaxInclusiveAmount")),
        prepaid: num(&get("LegalMonetaryTotal/PrepaidAmount")),
        payable: num(&get("LegalMonetaryTotal/PayableAmount")),
    })
}

/// Problems found: arithmetic that does not add up and identifiers that fail their check.
pub fn check(inv: &Invoice) -> Vec<String> {
    let mut p = Vec::new();
    let close = |a: f64, b: f64| (a - b).abs() <= 0.011;
    if inv.id.is_empty() {
        p.push("no invoice number (cbc:ID)".into());
    }
    if let (Some(n), Some(t), Some(g)) = (inv.net, inv.tax, inv.gross) {
        if !close(n + t, g) {
            p.push(format!("net {n:.2} + tax {t:.2} is not gross {g:.2}"));
        }
    }
    if let (Some(s), Some(n)) = (inv.line_sum, inv.net) {
        if !close(s, n) {
            p.push(format!(
                "line amounts add up to {s:.2}, document net is {n:.2}"
            ));
        }
    }
    if let (Some(g), Some(pay)) = (inv.gross, inv.payable) {
        let expected = g - inv.prepaid.unwrap_or(0.0);
        // a rounding amount may legitimately differ by a small amount
        if (expected - pay).abs() > 1.0 {
            p.push(format!(
                "payable {pay:.2} differs from gross - prepaid {expected:.2}"
            ));
        }
    }
    for (who, id) in [
        ("supplier", &inv.supplier.tax_id),
        ("customer", &inv.customer.tax_id),
    ] {
        if id.to_uppercase().starts_with("RO") && !cui_valid(id) {
            p.push(format!("{who} VAT id {id} fails the CUI check digit"));
        }
    }
    if !inv.customization.is_empty() && !inv.customization.to_lowercase().contains("cius-ro") {
        p.push(format!(
            "CustomizationID {:?} does not name CIUS-RO",
            inv.customization
        ));
    }
    p
}

/// XML text from a path: plain `.xml`, or the invoice XML inside ANAF's ZIP.
pub fn read_xml(path: &std::path::Path) -> Result<String> {
    let data = std::fs::read(path)?;
    if data.starts_with(b"PK") {
        let Some(list) = crate::zipmini::list(&data) else {
            bail!("unreadable ZIP")
        };
        for e in &list {
            let n = e.name.to_lowercase();
            if n.ends_with(".xml") && !n.contains("semnatura") && !n.contains("signature") {
                let Some(b) = crate::zipmini::read(&data, e) else {
                    bail!("cannot unpack {}", e.name)
                };
                return Ok(String::from_utf8_lossy(&b).into_owned());
            }
        }
        bail!("no invoice XML inside the ZIP");
    }
    Ok(String::from_utf8_lossy(&data).into_owned())
}

pub fn render(inv: &Invoice, problems: &[String]) -> String {
    let m = |v: Option<f64>| v.map(|x| format!("{x:.2}")).unwrap_or_else(|| "-".into());
    let mut s = format!(
        "{} {}  issued {}  due {}  {}\nsupplier: {} ({})\ncustomer: {} ({})\nlines: {}  net {}  tax {}  gross {}  payable {}\n",
        inv.kind, inv.id, inv.issue_date, if inv.due_date.is_empty() { "-" } else { &inv.due_date }, inv.currency,
        inv.supplier.name, inv.supplier.tax_id, inv.customer.name, inv.customer.tax_id,
        inv.lines, m(inv.net), m(inv.tax), m(inv.gross), m(inv.payable)
    );
    if !inv.iban.is_empty() {
        s += &format!("IBAN: {}\n", inv.iban);
    }
    if problems.is_empty() {
        s += "checks: amounts add up, identifiers pass\n";
    } else {
        for p in problems {
            s += &format!("PROBLEM: {p}\n");
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = r#"<?xml version="1.0"?>
<!-- sample -->
<Invoice xmlns="urn:oasis:names:specification:ubl:schema:xsd:Invoice-2" xmlns:cbc="urn:x" xmlns:cac="urn:y">
 <cbc:CustomizationID>urn:cen.eu:en16931:2017#compliant#urn:efactura.mfinante.ro:CIUS-RO:1.0.1</cbc:CustomizationID>
 <cbc:ID>FCT 0042</cbc:ID><cbc:IssueDate>2026-10-30</cbc:IssueDate><cbc:DueDate>2026-11-29</cbc:DueDate>
 <cbc:DocumentCurrencyCode>RON</cbc:DocumentCurrencyCode>
 <cac:AccountingSupplierParty><cac:Party>
   <cac:PartyTaxScheme><cbc:CompanyID>RO1000009</cbc:CompanyID></cac:PartyTaxScheme>
   <cac:PartyLegalEntity><cbc:RegistrationName>Furnizor &amp; Fiii SRL</cbc:RegistrationName></cac:PartyLegalEntity>
 </cac:Party></cac:AccountingSupplierParty>
 <cac:AccountingCustomerParty><cac:Party>
   <cac:PartyTaxScheme><cbc:CompanyID>RO1000018</cbc:CompanyID></cac:PartyTaxScheme>
   <cac:PartyLegalEntity><cbc:RegistrationName>Client SA</cbc:RegistrationName></cac:PartyLegalEntity>
 </cac:Party></cac:AccountingCustomerParty>
 <cac:PaymentMeans><cac:PayeeFinancialAccount><cbc:ID>RO49AAAA1B31007593840000</cbc:ID></cac:PayeeFinancialAccount></cac:PaymentMeans>
 <cac:TaxTotal><cbc:TaxAmount currencyID="RON">190.00</cbc:TaxAmount></cac:TaxTotal>
 <cac:LegalMonetaryTotal><cbc:LineExtensionAmount>1000.00</cbc:LineExtensionAmount>
   <cbc:TaxExclusiveAmount>1000.00</cbc:TaxExclusiveAmount><cbc:TaxInclusiveAmount>1190.00</cbc:TaxInclusiveAmount>
   <cbc:PayableAmount>1190.00</cbc:PayableAmount></cac:LegalMonetaryTotal>
 <cac:InvoiceLine><cbc:ID>1</cbc:ID><cbc:LineExtensionAmount>600.00</cbc:LineExtensionAmount></cac:InvoiceLine>
 <cac:InvoiceLine><cbc:ID>2</cbc:ID><cbc:LineExtensionAmount>400.00</cbc:LineExtensionAmount></cac:InvoiceLine>
</Invoice>"#;

    #[test]
    fn cui_check_digit() {
        // check digit computed with weights 3 2 1 7 5 3 2 over the digits before the last
        assert!(cui_valid("RO1000009"));
        assert!(cui_valid("1000009"));
        assert!(!cui_valid("RO1000008"));
        assert!(!cui_valid("RO12AB"));
    }

    #[test]
    fn reads_fields_and_checks() {
        let inv = parse(XML).unwrap();
        assert_eq!(inv.id, "FCT 0042");
        assert_eq!(inv.supplier.name, "Furnizor & Fiii SRL");
        assert_eq!(inv.lines, 2);
        assert_eq!(inv.payable, Some(1190.0));
        let p = check(&inv);
        // the sample customer id was made up: only that one may be reported
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("customer VAT id"));
        let mut bad = inv.clone();
        bad.gross = Some(1200.0);
        assert!(check(&bad)
            .iter()
            .any(|x| x.contains("net 1000.00 + tax 190.00")));
    }

    #[test]
    fn other_documents_are_refused() {
        assert!(parse("<html><body>x</body></html>").is_err());
    }
}
