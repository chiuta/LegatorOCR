//! Hostile project files and per-page settings (pre.41): the parser must answer every line
//! with a value or an error, never a panic, and what it accepts must survive a round trip.

use legatorocr_core::Settings;
use legatorocr_engine::project::Project;

const HEAD: &str = "legatorocr-project 1\n";

fn parse_sheet(lines: &str) -> Result<Project, String> {
    Project::parse(&format!("{HEAD}[sheet]\npath = a.jpg\n{lines}\n"), None)
        .map_err(|e| e.to_string())
}

#[test]
fn bad_page_lines_are_errors_not_panics() {
    for bad in [
        "page0.mode = gray",
        "page3.mode = gray",
        "page-1.mode = gray",
        "page.mode = gray",
        "page1. = gray",
        "page1.mode",
        "page1.mode =",
        "page1.nope = 1",
        "page1.page1.mode = gray",
        "page1.page_keystone = 1",
        "page1.page_keystone = a,b",
        "page1.page_keystone = nan,1",
        "page1.page_keystone = 1,inf",
        "page1.page_keystone = 31,0",
        "page1.page_keystone = 1,2,3",
        "page1.page_keystone = ,",
        "page2.keystone = 1e40,0",
        "page1.cut_angle = 5",
        "page1.dpi = -5",
        "page1.dpi = 99999999999999999999",
        "page99999999999999999999.mode = gray",
        "keystone = 1",
        "keystone = nan,nan",
        "keystone = 31,0",
        "keystone = 0,-31",
        "keystone = 1,2,3",
        "cut-angle = nan",
        "cut-angle = 21",
        "cut-angle = -21",
        "cut-angle = 1e99",
        "cut-angle = ",
    ] {
        match parse_sheet(bad) {
            Err(_) => {}
            Ok(p) => {
                // accepted: then it must have been harmless and must round-trip
                let text = p.to_text(None);
                Project::parse(&text, None).unwrap_or_else(|e| {
                    panic!("{bad:?} accepted but its text does not parse: {e}")
                });
            }
        }
    }
}

#[test]
fn sheet_wide_keys_are_refused_on_a_page_and_unknown_pages_are_ignored_like_any_unknown_key() {
    for k in [
        "split = auto",
        "auto_orient = true",
        "cut_angle = 5",
        "keystone = 1,1",
        "dpi = 300",
    ] {
        assert!(parse_sheet(&format!("page1.{k}")).is_err(), "page1.{k}");
        assert!(parse_sheet(&format!("page2.{k}")).is_err(), "page2.{k}");
    }
    // not page1 / page2: an unknown key, which this format has always skipped
    let p = parse_sheet("page3.mode = gray\npage0.mode = gray").unwrap();
    assert_eq!(p.sheets[0].1.page_pairs, [vec![], vec![]]);
}

#[test]
fn good_page_lines_round_trip_and_stay_per_page() {
    let p = parse_sheet(
        "keystone = 3.5, -2\ncut-angle = -12.5\npage1.mode = gray\npage2.mode = bw\npage2.page_keystone = 1.5,-0.5\n",
    )
    .unwrap();
    let text = p.to_text(None);
    let q = Project::parse(&text, None).unwrap();
    assert_eq!(
        text,
        q.to_text(None),
        "writing twice must give the same text"
    );
    let adj = &q.sheets[0].1;
    assert_eq!(adj.keystone, [3.5, -2.0]);
    assert_eq!(adj.cut_angle_deg, -12.5);
    let s = adj.apply(&q.settings);
    assert!(s.part_settings[0].is_some() && s.part_settings[1].is_some());
    assert_eq!(
        s.part_settings[1].as_ref().unwrap().page_keystone,
        [1.5, -0.5]
    );
    assert_eq!(
        s.part_settings[0].as_ref().unwrap().page_keystone,
        [0.0, 0.0]
    );
}

#[test]
fn page_lines_do_not_leak_into_other_sheets() {
    let t = format!(
        "{HEAD}[sheet]\npath = a.jpg\npage1.mode = gray\nkeystone = 4,4\n\n[sheet]\npath = b.jpg\n"
    );
    let p = Project::parse(&t, None).unwrap();
    let base = Settings::default();
    let b = p.sheets[1].1.apply(&base);
    assert!(b.part_settings[0].is_none() && b.part_settings[1].is_none());
    assert_eq!(b.sheet_keystone, [0.0, 0.0]);
}

#[test]
fn random_garbage_lines_never_panic() {
    let mut x: u64 = 0x1234_5678_9ABC_DEF1;
    let mut next = || {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        x
    };
    let keys = [
        "page1.",
        "page2.",
        "page",
        "",
        "keystone",
        "cut-angle",
        "page1.page_keystone",
        "work",
        "erase",
        "guide",
    ];
    let vals = [
        "",
        "1",
        "-1",
        "nan",
        "inf",
        "1,2",
        "1,2,3,4",
        "a",
        "1e999",
        ",",
        "= =",
        "\u{0}",
        "ß,ü",
        "1;2",
        "  7  ,  8  ",
    ];
    for _ in 0..4000 {
        let mut body = String::new();
        for _ in 0..(next() % 6) {
            let k = keys[(next() % keys.len() as u64) as usize];
            let v = vals[(next() % vals.len() as u64) as usize];
            let mid = if next() % 7 == 0 { "" } else { " = " };
            body.push_str(&format!(
                "{k}{}{mid}{v}\n",
                ["mode", "", "x", "page_keystone", "dpi"][(next() % 5) as usize]
            ));
        }
        if let Ok(p) = parse_sheet(&body) {
            let text = p.to_text(None);
            let q =
                Project::parse(&text, None).unwrap_or_else(|e| panic!("{body:?} -> {text:?}: {e}"));
            assert_eq!(text, q.to_text(None), "{body:?}");
        }
    }
}

#[test]
fn a_huge_hostile_file_is_handled() {
    let mut t = String::from(HEAD);
    for i in 0..3000 {
        t.push_str(&format!("[sheet]\npath = p{i}.jpg\npage1.mode = gray\npage2.page_keystone = 1,1\nkeystone = 1,1\n\n"));
    }
    let p = Project::parse(&t, None).unwrap();
    assert_eq!(p.sheets.len(), 3000);
    // one gigantic line
    let big = format!(
        "{HEAD}[sheet]\npath = a.jpg\nguide = {}\n",
        "1,1 ".repeat(200_000)
    );
    let _ = Project::parse(&big, None);
    let long_key = format!(
        "{HEAD}[sheet]\npath = a.jpg\npage1.{} = 1\n",
        "x".repeat(100_000)
    );
    assert!(parse_sheet(&long_key[HEAD.len() + "[sheet]\npath = a.jpg\n".len()..]).is_err());
}
