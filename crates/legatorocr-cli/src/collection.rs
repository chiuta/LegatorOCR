//! `legatorocr collection …`: newspapers, magazines and books gathered in one `.lgrcol` file,
//! processed in a batch and searched as a whole. The work is done by
//! `legatorocr_engine::collection`; this file is arguments and printing.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use legatorocr_engine::collection::{
    self as col, Collection, Index, Item, ItemEdit, Kind, RunConfig, Status,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// A collection gathers newspapers, magazines, books and the like: the sources of each (images,
/// a PDF, a folder, a saved .lgr project), its metadata, the profile to process it with and
/// where it stands. The file (.lgrcol, JSON) keeps paths relative to its own folder, so the
/// folder can be moved. Processed items are searchable by their OCR text.
#[derive(Parser)]
#[command(name = "legatorocr collection", version)]
struct CollectionCli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create an empty collection file (a folder gets NAME.lgrcol inside).
    New {
        /// The .lgrcol file to create, or an existing folder to create it in.
        target: PathBuf,
        /// Name of the collection (default: the file name).
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        description: Option<String>,
        /// Profile for items that name none (see `legatorocr --list-profiles`).
        #[arg(long)]
        profile: Option<String>,
        /// Do not recognise text while processing (the collection then cannot be searched).
        #[arg(long)]
        no_ocr: bool,
        /// Write the PDFs here instead of next to the collection file.
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Add items. Each PATH (a PDF, a .lgr project, a folder of page images) becomes one item
    /// whose kind, title, date and issue are guessed from its name, e.g.
    /// scinteia_tineretului_1977_09_8801.pdf is a newspaper, 1977-09, no. 8801.
    Add {
        file: PathBuf,
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Every PATH is a folder whose PDFs, .lgr projects and subfolders of images each become an item.
        #[arg(long)]
        folder: bool,
        /// Make ONE item of all the PATHs (for pages given as several image files).
        #[arg(long, conflicts_with = "folder")]
        as_one: bool,
        /// newspaper, magazine, book or other (instead of the guess).
        #[arg(long)]
        kind: Option<String>,
        /// Title (instead of the guess; only with one item).
        #[arg(long)]
        title: Option<String>,
        /// Language codes of the text, e.g. ro+fr (chooses the OCR language).
        #[arg(long)]
        lang: Option<String>,
        /// Profile for the new items.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List the items, one per line (K = N newspaper, M magazine, B book, O other).
    List {
        file: PathBuf,
        /// Only items with this status: todo, processed, ocr_done or checked.
        #[arg(long)]
        status: Option<String>,
    },
    /// Process the items that are not done yet (status todo) with their profiles; one that
    /// fails is reported and the others go on. Results are saved after each item.
    Run {
        file: PathBuf,
        /// Only these items (ids from `list`; repeat the option). Done items are processed again.
        #[arg(long = "item", value_name = "ID")]
        items: Vec<String>,
        /// Also retry the items whose last run failed.
        #[arg(long)]
        retry_failed: bool,
        /// Write the PDFs here (default: the collection's output folder, else next to the file).
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Do not recognise text this time.
        #[arg(long)]
        no_ocr: bool,
        /// Profile for items that name none.
        #[arg(long)]
        profile: Option<String>,
        /// Tesseract program to use.
        #[arg(long)]
        tesseract: Option<PathBuf>,
    },
    /// Change the details of one item: title, issue, date, kind, profile, status, tags, notes.
    /// Only the options given are changed; nothing is changed if one value is not valid.
    Set {
        file: PathBuf,
        /// Item id (from `list`).
        item: String,
        #[arg(long)]
        title: Option<String>,
        /// Issue, number or volume as written (`8801`, `vol. 3`); empty clears it.
        #[arg(long)]
        issue: Option<String>,
        /// YYYY, YYYY-MM or YYYY-MM-DD; empty clears it.
        #[arg(long)]
        date: Option<String>,
        /// newspaper, magazine, book or other.
        #[arg(long)]
        kind: Option<String>,
        /// Profile to process with (see `legatorocr --list-profiles`); empty = the collection's.
        #[arg(long)]
        profile: Option<String>,
        /// todo, processed, ocr_done or checked. `todo` makes the next `run` process it again.
        #[arg(long)]
        status: Option<String>,
        /// Comma-separated; replaces the tags.
        #[arg(long)]
        tags: Option<String>,
        #[arg(long)]
        notes: Option<String>,
    },
    /// Take items out of the collection and out of the search index. The PDFs and the source
    /// files are not touched.
    Remove {
        file: PathBuf,
        #[arg(required = true)]
        items: Vec<String>,
    },
    /// Read the text layer of the items' PDFs into the search index without processing
    /// anything again: for items whose source is a PDF that already has text, and for items
    /// processed earlier. Default: every item that has a PDF to read. A scan without text is
    /// reported and not indexed (process it with OCR instead).
    #[command(alias = "index")]
    Reindex {
        file: PathBuf,
        /// Only these items (ids from `list`; repeat the option).
        #[arg(long = "item", value_name = "ID")]
        items: Vec<String>,
    },
    /// Search the recognised text of all processed items: item, page and a short snippet.
    /// Case, diacritics and the old Romanian spelling do not matter (Scînteia finds Scânteia).
    Search {
        file: PathBuf,
        query: String,
        /// Stop after this many hits.
        #[arg(long, default_value_t = 50)]
        max: usize,
    },
}

/// True when the command line is `legatorocr collection …`.
pub fn wanted() -> bool {
    std::env::args().nth(1).as_deref() == Some("collection")
}

pub fn main() -> Result<()> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let cli = CollectionCli::try_parse_from(args).unwrap_or_else(|e| e.exit());
    match cli.cmd {
        Cmd::New {
            target,
            name,
            description,
            profile,
            no_ocr,
            out,
        } => new(&target, name, description, profile, no_ocr, out),
        Cmd::Add {
            file,
            paths,
            folder,
            as_one,
            kind,
            title,
            lang,
            profile,
        } => add(&file, &paths, folder, as_one, kind, title, lang, profile),
        Cmd::List { file, status } => list(&file, status),
        Cmd::Run {
            file,
            items,
            retry_failed,
            out,
            no_ocr,
            profile,
            tesseract,
        } => run(&file, items, retry_failed, out, no_ocr, profile, tesseract),
        Cmd::Set {
            file,
            item,
            title,
            issue,
            date,
            kind,
            profile,
            status,
            tags,
            notes,
        } => {
            let e = ItemEdit {
                title,
                issue,
                date,
                kind,
                profile,
                status,
                tags,
                notes,
            };
            set(&file, &item, &e)
        }
        Cmd::Remove { file, items } => remove(&file, &items),
        Cmd::Reindex { file, items } => reindex(&file, items),
        Cmd::Search { file, query, max } => search(&file, &query, max),
    }
}

fn check_profile(name: &str) -> Result<()> {
    legatorocr_engine::profiles::load(name)
        .map(|_| ())
        .with_context(|| format!("profile '{name}'"))
}

fn new(
    target: &Path,
    name: Option<String>,
    description: Option<String>,
    profile: Option<String>,
    no_ocr: bool,
    out: Option<PathBuf>,
) -> Result<()> {
    let file = if target.is_dir() {
        let n = name.clone().unwrap_or_else(|| "collection".into());
        target.join(format!("{}.{}", col::safe_stem(&n, "collection"), col::EXT))
    } else {
        target.to_path_buf()
    };
    if file.exists() {
        bail!("{}: already exists (not overwritten)", file.display());
    }
    let mut c = Collection::new(&name.unwrap_or_else(|| {
        file.file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default()
    }));
    c.description = description.unwrap_or_default();
    if let Some(p) = profile {
        check_profile(&p)?;
        c.default_profile = p;
    }
    c.ocr = !no_ocr;
    if let Some(o) = out {
        c.output_dir = col::to_stored(file.parent().unwrap_or(Path::new(".")), &o);
    }
    c.save(&file)?;
    println!("collection created: {}", file.display());
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn add(
    file: &Path,
    paths: &[PathBuf],
    folder: bool,
    as_one: bool,
    kind: Option<String>,
    title: Option<String>,
    lang: Option<String>,
    profile: Option<String>,
) -> Result<()> {
    let mut c = Collection::load(file)?;
    let kind = kind
        .map(|k| {
            Kind::from_code(&k)
                .with_context(|| format!("unknown kind '{k}' (newspaper, magazine, book, other)"))
        })
        .transpose()?;
    if let Some(p) = &profile {
        check_profile(p)?;
    }
    let mut added: Vec<String> = Vec::new();
    let (mut dup, mut ignored) = (0, 0);
    if folder {
        for p in paths {
            let r = c.add_folder(file, p)?;
            added.extend(r.added);
            dup += r.duplicates;
            ignored += r.ignored;
        }
    } else if as_one {
        added.push(c.add_item(file, paths)?);
    } else {
        for p in paths {
            added.push(c.add_item(file, std::slice::from_ref(p))?);
        }
    }
    if title.is_some() && added.len() > 1 {
        bail!("--title names one item, but {} would be added", added.len());
    }
    for id in &added {
        let it = c.item_mut(id).context("item vanished")?;
        if let Some(k) = kind {
            it.kind = k;
        }
        if let Some(t) = &title {
            it.title = t.clone();
        }
        if let Some(l) = &lang {
            it.languages = l
                .split(['+', ','])
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
        }
        if let Some(p) = &profile {
            it.profile = p.clone();
        }
    }
    c.save(file)?;
    for id in &added {
        if let Some(it) = c.item(id) {
            println!("{}", row(it));
        }
    }
    println!("{} item(s) added", added.len());
    if dup > 0 {
        println!("{dup} already in the collection, left as they were");
    }
    if ignored > 0 {
        println!(
            "{ignored} entry(ies) of the folder skipped (not a PDF, project or folder of images)"
        );
    }
    Ok(())
}

fn row(it: &Item) -> String {
    let pages = it.pages.map_or("-".to_string(), |p| p.to_string());
    format!(
        "{:<5} {} {:<9} {:>5}p  {}{}{}",
        it.id,
        it.kind.letter(),
        it.status.code(),
        pages,
        it.title,
        if it.issue_line().is_empty() {
            String::new()
        } else {
            format!("  [{}]", it.issue_line())
        },
        if it.error.is_empty() {
            String::new()
        } else {
            format!("  !! {}", it.error)
        },
    )
}

fn list(file: &Path, status: Option<String>) -> Result<()> {
    let c = Collection::load(file)?;
    let want = status
        .map(|s| {
            Status::from_code(&s).with_context(|| {
                format!("unknown status '{s}' (todo, processed, ocr_done, checked)")
            })
        })
        .transpose()?;
    println!("{} - {} item(s)", c.name, c.items.len());
    for it in c
        .items
        .iter()
        .filter(|i| want.is_none_or(|w| i.status == w))
    {
        println!("{}", row(it));
    }
    Ok(())
}

fn run(
    file: &Path,
    items: Vec<String>,
    retry_failed: bool,
    out: Option<PathBuf>,
    no_ocr: bool,
    profile: Option<String>,
    tesseract: Option<PathBuf>,
) -> Result<()> {
    let mut c = Collection::load(file)?;
    let mut ix = Index::load(&col::index_path(file))?;
    if let Some(p) = &profile {
        check_profile(p)?;
    }
    let ids = if items.is_empty() {
        c.pending_ids(retry_failed)
    } else {
        if let Some(bad) = items.iter().find(|i| c.item(i).is_none()) {
            bail!("no item '{bad}' in the collection (see `legatorocr collection list`)");
        }
        items
    };
    if ids.is_empty() {
        println!("nothing to do: no item is waiting (status todo)");
        return Ok(());
    }
    let cfg = RunConfig {
        out_dir: out,
        ocr: no_ocr.then_some(false),
        profile,
        tesseract,
    };
    let cancel = AtomicBool::new(false);
    let rep = col::run_batch(
        file,
        &mut c,
        &mut ix,
        &ids,
        &cfg,
        &cancel,
        &col::run_item_with_engine,
        &mut |n, total, title| {
            if n < total {
                eprintln!("[{}/{}] {}", n + 1, total, title);
            }
        },
    )?;
    for (id, msg) in &rep.failed {
        let t = c.item(id).map_or("", |i| i.title.as_str());
        eprintln!("FAILED {id} {t}: {msg}");
    }
    println!("{} processed, {} failed", rep.ok.len(), rep.failed.len());
    if !rep.failed.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}

fn set(file: &Path, id: &str, e: &ItemEdit) -> Result<()> {
    if e.is_empty() {
        bail!("nothing to change: give at least one of --title --issue --date --kind --profile --status --tags --notes");
    }
    if let Some(p) = e
        .profile
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        check_profile(p)?;
    }
    let mut c = Collection::load(file)?;
    let it = c.item_mut(id).with_context(|| {
        format!("no item '{id}' in the collection (see `legatorocr collection list`)")
    })?;
    it.apply(e)?;
    let line = row(it);
    c.save(file)?;
    println!("{line}");
    Ok(())
}

fn remove(file: &Path, ids: &[String]) -> Result<()> {
    let mut c = Collection::load(file)?;
    if let Some(bad) = ids.iter().find(|i| c.item(i).is_none()) {
        bail!("no item '{bad}' in the collection (see `legatorocr collection list`)");
    }
    let idx = col::index_path(file);
    let mut ix = Index::load(&idx)?;
    let mut in_index = false;
    for id in ids {
        c.remove(id);
        in_index |= ix.has(id);
        ix.remove(id);
    }
    c.save(file)?;
    if in_index {
        ix.save(&idx)?;
    }
    println!(
        "{} item(s) removed (PDFs and sources left as they were)",
        ids.len()
    );
    Ok(())
}

fn reindex(file: &Path, items: Vec<String>) -> Result<()> {
    let mut c = Collection::load(file)?;
    let mut ix = Index::load(&col::index_path(file))?;
    let ids: Vec<String> = if items.is_empty() {
        let base = file.parent().unwrap_or(Path::new("."));
        c.items
            .iter()
            .filter(|i| i.text_pdf(base).is_some())
            .map(|i| i.id.clone())
            .collect()
    } else {
        if let Some(bad) = items.iter().find(|i| c.item(i).is_none()) {
            bail!("no item '{bad}' in the collection (see `legatorocr collection list`)");
        }
        items
    };
    if ids.is_empty() {
        println!("nothing to do: no item has a PDF to read");
        return Ok(());
    }
    let cancel = AtomicBool::new(false);
    let rep = col::reindex(
        file,
        &mut c,
        &mut ix,
        &ids,
        &cancel,
        &mut |n, total, title| {
            if n < total {
                eprintln!("[{}/{}] {}", n + 1, total, title);
            }
        },
    )?;
    for id in &rep.no_text {
        let t = c.item(id).map_or("", |i| i.title.as_str());
        eprintln!("no text layer {id} {t}: a scan; process it with OCR (`run --item {id}`)");
    }
    for (id, msg) in &rep.failed {
        let t = c.item(id).map_or("", |i| i.title.as_str());
        eprintln!("FAILED {id} {t}: {msg}");
    }
    println!(
        "{} indexed ({} pages), {} without text, {} failed",
        rep.indexed.len(),
        rep.pages,
        rep.no_text.len(),
        rep.failed.len()
    );
    if !rep.failed.is_empty() {
        std::process::exit(2);
    }
    Ok(())
}

fn search(file: &Path, query: &str, max: usize) -> Result<()> {
    let c = Collection::load(file)?;
    let ix = Index::load(&col::index_path(file))?;
    if ix.page_count() == 0 {
        eprintln!("note: nothing is indexed yet; process items with OCR (`collection run`) or read the text of PDFs that have it (`collection reindex`)");
    }
    let hits = ix.search(query, max);
    for h in &hits {
        let t = c.item(&h.item_id).map_or("?", |i| i.title.as_str());
        let l = c
            .item(&h.item_id)
            .map(|i| i.issue_line())
            .unwrap_or_default();
        println!(
            "{} {} [{}] p. {}{}: {}",
            h.item_id,
            t,
            l,
            h.page,
            if h.loose { " (words apart)" } else { "" },
            h.snippet
        );
    }
    println!("{} hit(s)", hits.len());
    Ok(())
}
