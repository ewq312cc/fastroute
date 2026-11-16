//! Plumbing shared by the subcommands (`route`, `render`, `inspect`, `ai-pack`): option
//! parsing with one help format, exit codes and checked output writing.
//!
//! The legacy flag interface (`-de board.dsn -do board.ses ...`) does not go through here; it
//! keeps its own parser in `main.rs` so existing scripts behave as before.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Exit codes of the subcommands (`docs/CLI.md`); 0 is success.
pub mod exit {
    /// The command failed (anything not covered below).
    pub const FAILED: u8 = 1;
    /// Wrong command line: unknown option, missing value, conflicting options.
    pub const USAGE: u8 = 2;
    /// An input file is missing, unreadable or not what it should be.
    pub const INPUT: u8 = 3;
    /// The output cannot be written (or would overwrite something without `--force`).
    pub const OUTPUT: u8 = 4;
    /// `route` finished and wrote its result, but connections are still unrouted.
    pub const PARTIAL: u8 = 5;
}

#[derive(Debug, Clone, PartialEq)]
pub struct CmdError {
    pub message: String,
    pub code: u8,
}

impl CmdError {
    pub fn new(code: u8, message: impl Into<String>) -> CmdError {
        CmdError { message: message.into(), code }
    }
    pub fn usage(message: impl Into<String>) -> CmdError {
        CmdError::new(exit::USAGE, message)
    }
    pub fn input(message: impl Into<String>) -> CmdError {
        CmdError::new(exit::INPUT, message)
    }
    pub fn output(message: impl Into<String>) -> CmdError {
        CmdError::new(exit::OUTPUT, message)
    }
    pub fn failed(message: impl Into<String>) -> CmdError {
        CmdError::new(exit::FAILED, message)
    }
}

impl From<fr_render::RenderError> for CmdError {
    fn from(e: fr_render::RenderError) -> CmdError {
        use fr_render::RenderError as E;
        let code = match e {
            E::Io(_) => exit::OUTPUT,
            E::NoOutline | E::Encode(_) => exit::FAILED,
            _ => exit::USAGE,
        };
        CmdError::new(code, e.to_string())
    }
}

/// One option of a command.
#[derive(Clone, Copy)]
pub struct Opt {
    pub long: &'static str,
    pub short: Option<char>,
    /// The name of the value in the help (`FILE`, `N`, ...); `None` for a flag.
    pub value: Option<&'static str>,
    pub help: &'static str,
}

pub const fn opt(long: &'static str, short: Option<char>, value: Option<&'static str>, help: &'static str) -> Opt {
    Opt { long, short, value, help }
}

/// A subcommand: its name, usage and options. The help text and the parser come from this.
pub struct Spec {
    pub name: &'static str,
    pub about: &'static str,
    pub usage: &'static str,
    pub opts: &'static [Opt],
    /// Arguments that start with one of these are not parsed but collected unchanged in
    /// [`Parsed::passthrough`] (`--router.autorouter.max_passes=3`).
    pub passthrough: &'static [&'static str],
    pub footer: &'static str,
}

#[derive(Debug, Default)]
pub struct Parsed {
    values: HashMap<&'static str, Vec<String>>,
    flags: std::collections::HashSet<&'static str>,
    pub positional: Vec<String>,
    /// Arguments matched by [`Spec::passthrough`], in order.
    pub passthrough: Vec<String>,
    /// `-h` / `--help` was given.
    pub help: bool,
}

impl Parsed {
    /// The last value of an option.
    pub fn value(&self, long: &str) -> Option<&str> {
        self.values.get(long).and_then(|v| v.last()).map(String::as_str)
    }
    /// Every value of a repeatable option, in order.
    pub fn values(&self, long: &str) -> Vec<&str> {
        self.values.get(long).map(|v| v.iter().map(String::as_str).collect()).unwrap_or_default()
    }
    pub fn flag(&self, long: &str) -> bool {
        self.flags.contains(long)
    }
    pub fn has(&self, long: &str) -> bool {
        self.flag(long) || self.values.contains_key(long)
    }
    pub fn path(&self, long: &str) -> Option<PathBuf> {
        self.value(long).map(PathBuf::from)
    }
    pub fn f64(&self, long: &str) -> Result<Option<f64>, CmdError> {
        match self.value(long) {
            None => Ok(None),
            Some(v) => v.trim().parse::<f64>().ok().filter(|x| x.is_finite()).map(Some).ok_or_else(|| CmdError::usage(format!("--{long}: '{v}' is not a number"))),
        }
    }
}

impl Spec {
    pub fn help_text(&self) -> String {
        let mut out = format!("fastroute {} - {}\n\nusage: fastroute {}\n\noptions:\n", self.name, self.about, self.usage);
        let left = |o: &Opt| {
            let mut s = String::from("  ");
            match o.short {
                Some(c) => s.push_str(&format!("-{c}, ")),
                None => s.push_str("    "),
            }
            s.push_str(&format!("--{}", o.long));
            if let Some(v) = o.value {
                s.push_str(&format!(" {v}"));
            }
            s
        };
        let width = self.opts.iter().map(|o| left(o).len()).max().unwrap_or(0).max(24) + 2;
        for o in self.opts {
            let l = left(o);
            let mut lines = o.help.lines();
            out.push_str(&format!("{l:<width$}{}\n", lines.next().unwrap_or("")));
            for more in lines {
                out.push_str(&format!("{:<width$}{more}\n", ""));
            }
        }
        out.push_str(&format!("{:<width$}show this help\n", "  -h, --help"));
        if !self.footer.is_empty() {
            out.push('\n');
            out.push_str(self.footer);
            out.push('\n');
        }
        out.push_str("\nexit codes: 0 ok, 1 failed, 2 usage, 3 input, 4 output, 5 partial result (see docs/CLI.md)\n");
        out
    }

    /// Parses `args` (without the subcommand name). `--opt value` and `--opt=value` both work;
    /// `--` ends the options.
    pub fn parse(&self, args: &[String]) -> Result<Parsed, CmdError> {
        let mut p = Parsed::default();
        let mut i = 0;
        let unknown = |a: &str| CmdError::usage(format!("unknown option '{a}' (see `fastroute {} --help`)", self.name));
        while i < args.len() {
            let a = &args[i];
            i += 1;
            if a == "--" {
                p.positional.extend(args[i..].iter().cloned());
                break;
            }
            if a == "-h" || a == "--help" {
                p.help = true;
                continue;
            }
            if self.passthrough.iter().any(|pre| a.starts_with(pre)) {
                p.passthrough.push(a.clone());
                continue;
            }
            let (opt, inline): (&Opt, Option<String>) = if let Some(rest) = a.strip_prefix("--") {
                let (name, inline) = match rest.split_once('=') {
                    Some((n, v)) => (n, Some(v.to_string())),
                    None => (rest, None),
                };
                (self.opts.iter().find(|o| o.long == name).ok_or_else(|| unknown(a))?, inline)
            } else if a.len() >= 2 && a.starts_with('-') && !a[1..].starts_with(|c: char| c.is_ascii_digit() || c == '.') {
                let mut chars = a[1..].chars();
                let c = chars.next().unwrap();
                let rest: String = chars.collect();
                let o = self.opts.iter().find(|o| o.short == Some(c)).ok_or_else(|| unknown(a))?;
                (o, if rest.is_empty() { None } else { Some(rest.trim_start_matches('=').to_string()) })
            } else {
                p.positional.push(a.clone());
                continue;
            };
            match opt.value {
                None => {
                    if inline.is_some() {
                        return Err(CmdError::usage(format!("--{} takes no value", opt.long)));
                    }
                    p.flags.insert(opt.long);
                }
                Some(meta) => {
                    let v = match inline {
                        Some(v) => v,
                        None => {
                            let v = args.get(i).ok_or_else(|| CmdError::usage(format!("--{} needs a value ({meta})", opt.long)))?;
                            i += 1;
                            v.clone()
                        }
                    };
                    p.values.entry(opt.long).or_default().push(v);
                }
            }
        }
        Ok(p)
    }
}

/// The absolute, `..`-free form of a path, also for files that do not exist yet.
fn absolute(path: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(path) {
        return c;
    }
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let (dir, file) = (abs.parent().map(Path::to_path_buf).unwrap_or_default(), abs.file_name().map(|f| f.to_owned()));
    let dir = std::fs::canonicalize(&dir).unwrap_or(dir);
    match file {
        Some(f) => dir.join(f),
        None => dir,
    }
}

pub fn same_file(a: &Path, b: &Path) -> bool {
    absolute(a) == absolute(b)
}

/// Checks that `out` may be written: not one of the `inputs`, and not existing unless `force`.
pub fn check_output(out: &Path, inputs: &[&Path], force: bool) -> Result<(), CmdError> {
    for i in inputs {
        if same_file(out, i) {
            return Err(CmdError::output(format!("{}: the output is the input file; fastroute never overwrites its input", out.display())));
        }
    }
    if out.is_dir() {
        return Err(CmdError::output(format!("{}: is a directory", out.display())));
    }
    if out.exists() && !force {
        return Err(CmdError::output(format!("{}: already exists (use --force to overwrite it)", out.display())));
    }
    match out.parent() {
        Some(d) if !d.as_os_str().is_empty() && !d.is_dir() => Err(CmdError::output(format!("{}: the directory {} does not exist", out.display(), d.display()))),
        _ => Ok(()),
    }
}

/// Writes through a temporary file and a rename, after [`check_output`].
pub fn write_output(out: &Path, bytes: &[u8], inputs: &[&Path], force: bool) -> Result<(), CmdError> {
    check_output(out, inputs, force)?;
    write_atomically(out, bytes)
}

pub fn write_atomically(out: &Path, bytes: &[u8]) -> Result<(), CmdError> {
    let mut tmp = out.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| CmdError::output(format!("{}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, out).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        CmdError::output(format!("{}: {e}", out.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPTS: &[Opt] = &[
        opt("input", Some('i'), Some("FILE"), "input design"),
        opt("net", None, Some("NAME"), "a net (repeatable)"),
        opt("force", None, None, "overwrite"),
        opt("quiet", Some('q'), None, "no progress"),
        opt("scale", None, Some("N"), "a number"),
    ];
    const SPEC: Spec = Spec { name: "demo", about: "a demo", usage: "demo --input FILE [options]", opts: OPTS, passthrough: &["--router."], footer: "" };

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_long_short_inline_repeated_and_flags() {
        let p = SPEC.parse(&args(&["-i", "a.dsn", "--net=A", "--net", "B", "--force", "-q", "rest", "--scale", "-3"])).unwrap();
        assert_eq!(p.value("input"), Some("a.dsn"));
        assert_eq!(p.values("net"), vec!["A", "B"]);
        assert!(p.flag("force") && p.flag("quiet") && !p.flag("input"));
        assert_eq!(p.positional, vec!["rest"]);
        assert_eq!(p.f64("scale").unwrap(), Some(-3.0));
        assert_eq!(SPEC.parse(&args(&["-ia.dsn"])).unwrap().value("input"), Some("a.dsn"));
        assert_eq!(SPEC.parse(&args(&["--", "--force"])).unwrap().positional, vec!["--force"]);
        // settings of the router are passed on as they are, also with a value in the next word
        let p = SPEC.parse(&args(&["--router.optimizer.enabled=false", "-i", "a", "--router.x=1"])).unwrap();
        assert_eq!(p.passthrough, vec!["--router.optimizer.enabled=false", "--router.x=1"]);
        assert_eq!(p.value("input"), Some("a"));
    }

    #[test]
    fn usage_errors() {
        for (a, needle) in [
            (vec!["--nope"], "unknown option '--nope'"),
            (vec!["-x"], "unknown option '-x'"),
            (vec!["--input"], "needs a value"),
            (vec!["--force=1"], "takes no value"),
        ] {
            let e = SPEC.parse(&args(&a)).unwrap_err();
            assert_eq!(e.code, exit::USAGE);
            assert!(e.message.contains(needle), "{}", e.message);
        }
        let p = SPEC.parse(&args(&["--scale", "abc"])).unwrap();
        assert_eq!(p.f64("scale").unwrap_err().code, exit::USAGE);
        assert!(SPEC.parse(&args(&["--help"])).unwrap().help);
    }

    #[test]
    fn help_lists_every_option() {
        let h = SPEC.help_text();
        for needle in ["--input FILE", "-i,", "--net NAME", "--force", "-q, --quiet", "--help", "exit codes"] {
            assert!(h.contains(needle), "{needle}\n{h}");
        }
    }

    #[test]
    fn output_checks() {
        let dir = std::env::temp_dir().join(format!("fastroute-cmd-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("in.dsn");
        let out = dir.join("out.png");
        std::fs::write(&input, b"x").unwrap();
        // not the input, even spelled differently
        let alias = dir.join("sub/../in.dsn");
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        assert!(check_output(&alias, &[&input], true).unwrap_err().message.contains("never overwrites its input"));
        // new file is fine; an existing one needs --force
        assert!(check_output(&out, &[&input], false).is_ok());
        write_output(&out, b"1", &[&input], false).unwrap();
        assert_eq!(check_output(&out, &[&input], false).unwrap_err().code, exit::OUTPUT);
        assert!(check_output(&out, &[&input], false).unwrap_err().message.contains("--force"));
        write_output(&out, b"2", &[&input], true).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"2");
        assert!(!dir.join("out.png.tmp").exists());
        // missing directory, and a directory as output
        assert!(check_output(&dir.join("nope/x.png"), &[], false).unwrap_err().message.contains("does not exist"));
        assert!(check_output(&dir, &[], true).unwrap_err().message.contains("is a directory"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
