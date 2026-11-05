//! `--install-modules`: one command that installs the optional helpers.
//!
//! Nothing here is needed to run LegatorOCR (the native OCR engine is built in). The modules
//! only add: Tesseract with the eight interface languages, the PDF renderers used for the
//! few cases the native reader does not decode, qpdf, and the Python side of the ONNX bridge.
//! `plan` is pure (it only builds command lines); `run` executes them one by one and keeps
//! going when one fails, so a package name that a distribution lacks never blocks the rest.

use crate::doctor::Check;
use anyhow::{bail, Result};
use std::process::Command;

/// An optional group of helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    /// Tesseract, its orientation data and the eight interface-language models.
    Tesseract,
    /// pdftoppm (poppler), mutool (MuPDF) and Ghostscript.
    Renderers,
    /// qpdf.
    Qpdf,
    /// Python packages of `scripts/engine-onnx.py` (onnxruntime, numpy, Pillow).
    Onnx,
}

pub const ALL: [Module; 4] = [
    Module::Tesseract,
    Module::Renderers,
    Module::Qpdf,
    Module::Onnx,
];

impl Module {
    pub fn name(self) -> &'static str {
        match self {
            Module::Tesseract => "tesseract",
            Module::Renderers => "renderers",
            Module::Qpdf => "qpdf",
            Module::Onnx => "onnx",
        }
    }
}

/// A package manager the plan can be written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Manager {
    Apt,
    Dnf,
    Pacman,
    Brew,
    Winget,
}

impl Manager {
    pub fn name(self) -> &'static str {
        match self {
            Manager::Apt => "apt",
            Manager::Dnf => "dnf",
            Manager::Pacman => "pacman",
            Manager::Brew => "brew",
            Manager::Winget => "winget",
        }
    }
    pub fn parse(s: &str) -> Option<Manager> {
        [
            Manager::Apt,
            Manager::Dnf,
            Manager::Pacman,
            Manager::Brew,
            Manager::Winget,
        ]
        .into_iter()
        .find(|m| m.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// One command of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    pub module: Module,
    pub program: String,
    pub args: Vec<String>,
    /// Needs administrator rights (`sudo` is put in front when not already root).
    pub root: bool,
}

impl Step {
    fn new(module: Module, root: bool, program: &str, args: &[&str]) -> Step {
        Step {
            module,
            program: program.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            root,
        }
    }
    /// The command as a person would type it.
    pub fn display(&self, sudo: bool) -> String {
        let mut s = String::new();
        if self.root && sudo {
            s.push_str("sudo ");
        }
        s.push_str(&self.program);
        for a in &self.args {
            s.push(' ');
            s.push_str(a);
        }
        s
    }
}

/// Parse `all`, `missing` (resolved by the caller) or a comma list such as `tesseract,qpdf`.
/// Returns `None` for `missing` so the caller can ask the doctor.
pub fn parse_modules(list: &str) -> Result<Option<Vec<Module>>> {
    let l = list.trim().to_ascii_lowercase();
    if l.is_empty() || l == "all" {
        return Ok(Some(ALL.to_vec()));
    }
    if l == "missing" {
        return Ok(None);
    }
    let mut v = Vec::new();
    for part in l.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match ALL.iter().find(|m| m.name() == part) {
            Some(m) if !v.contains(m) => v.push(*m),
            Some(_) => {}
            None => bail!(
                "unknown module '{part}' (tesseract, renderers, qpdf, onnx, or all / missing)"
            ),
        }
    }
    Ok(Some(v))
}

/// Modules the doctor reports as incomplete.
pub fn missing_modules(checks: &[Check]) -> Vec<Module> {
    let bad = |pred: &dyn Fn(&str) -> bool| checks.iter().any(|c| !c.ok && pred(&c.name));
    let mut v = Vec::new();
    if bad(&|n| n == "tesseract" || n.starts_with("tesseract model")) {
        v.push(Module::Tesseract);
    }
    if bad(&|n| n == "pdftoppm" || n == "mutool" || n == "gs") {
        v.push(Module::Renderers);
    }
    if bad(&|n| n == "qpdf") {
        v.push(Module::Qpdf);
    }
    if bad(&|n| n.starts_with("onnx bridge")) {
        v.push(Module::Onnx);
    }
    v
}

const LANGS: [&str; 8] = ["ron", "eng", "fra", "ita", "spa", "por", "deu", "hun"];

fn with_prefix(prefix: &str, items: &[&str]) -> Vec<String> {
    items.iter().map(|i| format!("{prefix}{i}")).collect()
}

fn pip(module: Module, python: &str) -> Step {
    Step::new(
        module,
        false,
        python,
        &[
            "-m",
            "pip",
            "install",
            "--user",
            "--break-system-packages",
            "onnxruntime",
            "numpy",
            "pillow",
        ],
    )
}

/// The commands that install `modules` with `manager`, in order.
///
/// Package names: apt names were checked against an Ubuntu 24.04 index; the dnf, pacman,
/// Homebrew and winget names are written from the packages' published names and have not
/// been run here (a name that does not exist only fails its own step).
pub fn plan(modules: &[Module], manager: Manager) -> Vec<Step> {
    let mut v = Vec::new();
    if manager == Manager::Apt {
        v.push(Step::new(Module::Tesseract, true, "apt-get", &["update"]));
    }
    // Several packages in one command, except where the manager installs one per command.
    let install = |m: Module, pkgs: Vec<String>, root: bool| -> Step {
        let (prog, mut head): (&str, Vec<&str>) = match manager {
            Manager::Apt => ("apt-get", vec!["install", "-y"]),
            Manager::Dnf => ("dnf", vec!["install", "-y"]),
            Manager::Pacman => ("pacman", vec!["-S", "--needed", "--noconfirm"]),
            Manager::Brew => ("brew", vec!["install"]),
            Manager::Winget => ("winget", vec![]),
        };
        let mut args: Vec<String> = head.drain(..).map(String::from).collect();
        args.extend(pkgs);
        Step {
            module: m,
            program: prog.into(),
            args,
            root,
        }
    };
    let winget = |m: Module, id: &str| -> Step {
        Step::new(
            m,
            false,
            "winget",
            &[
                "install",
                "--id",
                id,
                "-e",
                "--silent",
                "--accept-package-agreements",
                "--accept-source-agreements",
            ],
        )
    };
    for &m in modules {
        match (m, manager) {
            (Module::Tesseract, Manager::Apt) => {
                let mut p = vec!["tesseract-ocr".to_string(), "tesseract-ocr-osd".into()];
                p.extend(with_prefix("tesseract-ocr-", &LANGS));
                v.push(install(m, p, true));
            }
            (Module::Tesseract, Manager::Dnf) => {
                let mut p = vec!["tesseract".to_string()];
                p.extend(with_prefix("tesseract-langpack-", &LANGS));
                v.push(install(m, p, true));
            }
            (Module::Tesseract, Manager::Pacman) => {
                let mut p = vec!["tesseract".to_string(), "tesseract-data-osd".into()];
                p.extend(with_prefix("tesseract-data-", &LANGS));
                v.push(install(m, p, true));
            }
            (Module::Tesseract, Manager::Brew) => v.push(install(
                m,
                vec!["tesseract".into(), "tesseract-lang".into()],
                false,
            )),
            (Module::Tesseract, Manager::Winget) => v.push(winget(m, "UB-Mannheim.TesseractOCR")),

            (Module::Renderers, Manager::Apt) => v.push(install(
                m,
                vec![
                    "poppler-utils".into(),
                    "mupdf-tools".into(),
                    "ghostscript".into(),
                ],
                true,
            )),
            (Module::Renderers, Manager::Dnf) => v.push(install(
                m,
                vec!["poppler-utils".into(), "mupdf".into(), "ghostscript".into()],
                true,
            )),
            (Module::Renderers, Manager::Pacman) => v.push(install(
                m,
                vec!["poppler".into(), "mupdf-tools".into(), "ghostscript".into()],
                true,
            )),
            (Module::Renderers, Manager::Brew) => v.push(install(
                m,
                vec!["poppler".into(), "mupdf-tools".into(), "ghostscript".into()],
                false,
            )),
            (Module::Renderers, Manager::Winget) => {
                v.push(winget(m, "oschwartz10612.Poppler"));
                v.push(winget(m, "ArtifexSoftware.GhostScript"));
            }

            (Module::Qpdf, Manager::Winget) => v.push(winget(m, "QPDF.QPDF")),
            (Module::Qpdf, Manager::Brew) => v.push(install(m, vec!["qpdf".into()], false)),
            (Module::Qpdf, _) => v.push(install(m, vec!["qpdf".into()], true)),

            (Module::Onnx, Manager::Apt) => {
                v.push(install(
                    m,
                    vec![
                        "python3-pip".into(),
                        "python3-numpy".into(),
                        "python3-pil".into(),
                    ],
                    true,
                ));
                v.push(pip(m, "python3"));
            }
            (Module::Onnx, Manager::Dnf) => {
                v.push(install(
                    m,
                    vec![
                        "python3-pip".into(),
                        "python3-numpy".into(),
                        "python3-pillow".into(),
                    ],
                    true,
                ));
                v.push(pip(m, "python3"));
            }
            (Module::Onnx, Manager::Pacman) => v.push(install(
                m,
                vec![
                    "python-numpy".into(),
                    "python-pillow".into(),
                    "python-onnxruntime".into(),
                ],
                true,
            )),
            (Module::Onnx, Manager::Brew) => v.push(pip(m, "python3")),
            (Module::Onnx, Manager::Winget) => {
                v.push(winget(m, "Python.Python.3.12"));
                v.push(pip(m, "py"));
            }
        }
    }
    // the apt index refresh is only worth running when an apt package follows
    if manager == Manager::Apt
        && !v
            .iter()
            .any(|s| s.program == "apt-get" && s.args[0] == "install")
    {
        v.retain(|s| !(s.program == "apt-get" && s.args[0] == "update"));
    }
    v
}

fn present(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

/// The package manager of this system, or `None` when none is recognised.
pub fn detect() -> Option<Manager> {
    let order: &[Manager] = if cfg!(windows) {
        &[Manager::Winget]
    } else if cfg!(target_os = "macos") {
        &[Manager::Brew]
    } else {
        &[Manager::Apt, Manager::Dnf, Manager::Pacman, Manager::Brew]
    };
    let probe = |m: Manager| match m {
        Manager::Apt => present("apt-get"),
        _ => present(m.name()),
    };
    order.iter().copied().find(|m| probe(*m))
}

fn is_root() -> bool {
    if cfg!(windows) {
        return true;
    }
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

/// Whether `sudo` has to go in front of root steps (not root, and `sudo` exists).
pub fn needs_sudo() -> bool {
    !is_root()
}

/// Result of one executed step.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub command: String,
    pub ok: bool,
    pub detail: String,
}

/// Run the steps one after another, output going straight to the terminal. A failing step is
/// recorded and the next one still runs. `on_start` is called with the command before each.
pub fn run(steps: &[Step], on_start: &mut dyn FnMut(&str)) -> Vec<Outcome> {
    let sudo = needs_sudo();
    steps
        .iter()
        .map(|s| {
            let shown = s.display(sudo);
            on_start(&shown);
            let (prog, args): (String, Vec<String>) = if s.root && sudo {
                let mut a = vec![s.program.clone()];
                a.extend(s.args.clone());
                ("sudo".into(), a)
            } else {
                (s.program.clone(), s.args.clone())
            };
            let mut cmd = Command::new(&prog);
            cmd.args(&args).env("DEBIAN_FRONTEND", "noninteractive");
            match cmd.status() {
                Ok(st) if st.success() => Outcome {
                    command: shown,
                    ok: true,
                    detail: "done".into(),
                },
                Ok(st) => Outcome {
                    command: shown,
                    ok: false,
                    detail: format!(
                        "exit status {}",
                        st.code().map_or("?".into(), |c| c.to_string())
                    ),
                },
                Err(e) => Outcome {
                    command: shown,
                    ok: false,
                    detail: format!("could not start: {e}"),
                },
            }
        })
        .collect()
}

/// Like [`run`] but nothing goes to the terminal: output is captured and the last lines of it
/// become the outcome's detail (for a window that has no terminal). A step that needs `sudo`
/// is not started (there is nobody to type the password) and is reported as failed.
pub fn run_captured(steps: &[Step]) -> Vec<Outcome> {
    let sudo = needs_sudo();
    steps
        .iter()
        .map(|s| {
            let shown = s.display(sudo);
            if s.root && sudo {
                return Outcome {
                    command: shown,
                    ok: false,
                    detail: "needs administrator rights: run it in a terminal".into(),
                };
            }
            match Command::new(&s.program)
                .args(&s.args)
                .env("DEBIAN_FRONTEND", "noninteractive")
                .stdin(std::process::Stdio::null())
                .output()
            {
                Ok(o) if o.status.success() => Outcome {
                    command: shown,
                    ok: true,
                    detail: "done".into(),
                },
                Ok(o) => {
                    let mut text = String::from_utf8_lossy(&o.stderr).into_owned();
                    if text.trim().is_empty() {
                        text = String::from_utf8_lossy(&o.stdout).into_owned();
                    }
                    let tail: Vec<&str> = text.lines().rev().take(3).collect();
                    let tail = tail.into_iter().rev().collect::<Vec<_>>().join(" | ");
                    Outcome {
                        command: shown,
                        ok: false,
                        detail: tail,
                    }
                }
                Err(e) => Outcome {
                    command: shown,
                    ok: false,
                    detail: format!("could not start: {e}"),
                },
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_lists_parse() {
        assert_eq!(parse_modules("all").unwrap().unwrap().len(), 4);
        assert_eq!(parse_modules("").unwrap().unwrap().len(), 4);
        assert_eq!(parse_modules("missing").unwrap(), None);
        assert_eq!(
            parse_modules("qpdf, tesseract,qpdf").unwrap().unwrap(),
            vec![Module::Qpdf, Module::Tesseract]
        );
        assert!(parse_modules("tesseract,wat").is_err());
    }

    #[test]
    fn apt_plan_installs_every_language_and_the_helpers() {
        let p = plan(&ALL, Manager::Apt);
        let text: Vec<String> = p.iter().map(|s| s.display(true)).collect();
        assert_eq!(text[0], "sudo apt-get update");
        let all = text.join("\n");
        for l in LANGS {
            assert!(all.contains(&format!("tesseract-ocr-{l}")), "{l}");
        }
        for pkg in [
            "tesseract-ocr-osd",
            "poppler-utils",
            "mupdf-tools",
            "ghostscript",
            "qpdf",
            "python3-numpy",
        ] {
            assert!(all.contains(pkg), "{pkg}");
        }
        assert!(all.contains("python3 -m pip install --user"));
        // the pip step runs as the user, never with sudo
        assert!(!all.contains("sudo python3"));
    }

    #[test]
    fn a_plan_without_apt_packages_has_no_index_refresh() {
        let p = plan(&[Module::Onnx], Manager::Pacman);
        assert!(p.iter().all(|s| s.program != "apt-get"));
        let p = plan(&[Module::Qpdf], Manager::Apt);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn brew_and_winget_do_not_use_sudo() {
        for m in [Manager::Brew, Manager::Winget] {
            assert!(plan(&ALL, m).iter().all(|s| !s.root), "{}", m.name());
        }
        let w = plan(&[Module::Tesseract], Manager::Winget);
        assert!(w[0].display(true).contains("UB-Mannheim.TesseractOCR"));
        assert!(w[0]
            .args
            .contains(&"--accept-package-agreements".to_string()));
    }

    #[test]
    fn missing_modules_follow_the_doctor() {
        let c = |n: &str, ok: bool| Check {
            name: n.into(),
            ok,
            detail: String::new(),
            hint: String::new(),
        };
        let checks = vec![
            c("legatorocr", true),
            c("tesseract", true),
            c("tesseract model hu (Hungarian)", false),
            c("pdftoppm", true),
            c("mutool", true),
            c("gs", true),
            c("qpdf", false),
            c("onnx bridge (python)", true),
        ];
        assert_eq!(
            missing_modules(&checks),
            vec![Module::Tesseract, Module::Qpdf]
        );
    }

    #[test]
    fn failed_steps_are_recorded_and_do_not_stop_the_rest() {
        let steps = vec![
            Step::new(Module::Qpdf, false, "definitely-not-a-program-xyz", &[]),
            Step::new(
                Module::Qpdf,
                false,
                if cfg!(windows) { "cmd" } else { "true" },
                &[],
            ),
        ];
        let mut seen = 0;
        let out = run(&steps, &mut |_| seen += 1);
        assert_eq!(seen, 2);
        assert!(!out[0].ok && out[0].detail.contains("could not start"));
        assert!(out[1].ok);
    }

    #[test]
    fn captured_run_keeps_the_tail_of_the_error() {
        if cfg!(windows) {
            return;
        }
        let steps = vec![Step::new(
            Module::Qpdf,
            false,
            "sh",
            &["-c", "echo one; echo bad thing >&2; exit 3"],
        )];
        let out = run_captured(&steps);
        assert!(!out[0].ok);
        assert!(out[0].detail.contains("bad thing"));
    }

    #[test]
    fn manager_names_parse_case_insensitively() {
        assert_eq!(Manager::parse("Brew"), Some(Manager::Brew));
        assert_eq!(Manager::parse("yum"), None);
    }
}
