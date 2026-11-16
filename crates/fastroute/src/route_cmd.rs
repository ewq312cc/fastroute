//! `fastroute route`: the routing command of the unified interface.
//!
//! * `route --input board.dsn` routes a Specctra design with the engine in this binary and
//!   writes a session (`.ses`), like the legacy `-de board.dsn -do board.ses`.
//! * `route --input board.kicad_pcb` routes a KiCad board through the KiCad adapter
//!   (`integrations/kicad/plugins/route_cli.py`, run with a Python that has KiCad's `pcbnew`
//!   module). That dependency is named when it is missing; nothing is guessed silently.
//!
//! Differences from the legacy flags, all deliberate: an output that exists is never replaced
//! without `--force` (checked before the long run starts), the default output never is the
//! input (`<name>.routed.ses` / `<name>.routed.kicad_pcb`), errors have exit codes, an
//! incomplete result is its own exit code (5), and `--json` prints one machine-readable summary.
//! The legacy interface is unchanged (see `docs/CLI.md`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde::Serialize;

use crate::cmd::{check_output, exit, opt, CmdError, Opt, Parsed, Spec};
use crate::load;

const OPTS: &[Opt] = &[
    opt("input", Some('i'), Some("FILE"), "design to route: Specctra .dsn (engine), or KiCad .kicad_pcb (through the KiCad adapter)\n(also the first positional argument)"),
    opt("output", Some('o'), Some("FILE"), "result: a session (.ses) for a .dsn, a board (.kicad_pcb) for a .kicad_pcb\n(default <input>.routed.ses / <input>.routed.kicad_pcb; the input is never the default)"),
    opt("force", None, None, "replace an existing output file (refused otherwise, before routing starts)"),
    opt("mode", None, Some("MODE"), "fast (default), exact (bit-identical to Freerouting's deterministic mode) or quick (no optimizer)"),
    opt("max-passes", None, Some("N"), "autorouter pass limit (0 = until done or stagnant)"),
    opt("max-time", None, Some("SECONDS"), "stop after this time and write the best result so far"),
    opt("threads", None, Some("N"), "optimizer threads"),
    opt("report", None, Some("FILE"), "write fastroute's JSON report of the result (.dsn input only)"),
    opt("diagnose", None, None, "with --report: classify each unrouted connection (congestion or blocked)"),
    opt("initial-session", None, Some("FILE"), "start from the wiring of a session (.dsn input only)"),
    opt("pairs", None, Some("FILE"), "differential pairs file (.dsn input only; see `fastroute --help`)"),
    opt("tune", None, Some("FILE"), "length matching file (.dsn input only)"),
    opt("pair-skew", None, Some("MM"), "allowed length difference within a pair (.dsn input only)"),
    opt("layer-heights", None, Some("MM,.."), "height of each copper layer (.dsn input only)"),
    opt("no-neckdown-classes", None, Some("A,B"), "net classes that keep their trace width at pins (.dsn input only)"),
    opt("multi-start", None, Some("N"), "rerun the autorouter with N-1 shuffled orders if connections stay unrouted"),
    opt("parity", None, None, "byte-identical Freerouting results (.dsn input only)"),
    opt("no-enhancements", None, None, "Freerouting's behaviour without fastroute's improvements (.dsn input only)"),
    opt("live", None, None, "watch the routing in the browser (opt-in; nothing else depends on it)"),
    opt("live-port", None, Some("PORT"), "port of --live (default 7878)"),
    opt("clear", None, None, "KiCad input: remove unlocked tracks and vias first"),
    opt("keep-existing", None, None, "KiCad input: keep all existing tracks, route only the open connections"),
    opt("python", None, Some("FILE"), "KiCad input: a Python that can import KiCad's pcbnew (default: $FASTROUTE_KICAD_PYTHON, KiCad's own, python3)"),
    opt("route-cli", None, Some("FILE"), "KiCad input: route_cli.py of the KiCad plugin (default: $FASTROUTE_ROUTE_CLI, next to this program)"),
    opt("work-dir", None, Some("DIR"), "KiCad input: keep the DSN / SES files in this directory"),
    opt("allow-partial", None, None, "exit 0 also when connections remain unrouted (default: exit code 5; the result is written either way)"),
    opt("json", None, None, "print one JSON summary on standard output (progress stays on standard error)"),
    opt("verbose", Some('v'), None, "debug output"),
    opt("quiet", Some('q'), None, "only warnings and errors"),
];

pub const SPEC: Spec = Spec {
    name: "route",
    about: "autoroute a Specctra design (.dsn) or a KiCad board (.kicad_pcb)",
    usage: "route --input board.dsn [--output board.ses] [options] [--router.<path>=<value> ...]\n       fastroute route --input board.kicad_pcb [--output routed.kicad_pcb] [options]",
    opts: OPTS,
    passthrough: &["--router."],
    footer: "Exit codes: 0 routed completely, 5 written but connections remain unrouted (0 with --allow-partial),\n\
             1 failed, 2 usage, 3 input, 4 output. The old interface (`fastroute -de board.dsn -do board.ses ...`)\n\
             still works unchanged; docs/CLI.md has the translation table.\n\
             KiCad boards need KiCad's Python (module pcbnew) and the plugin's route_cli.py; if either is\n\
             missing the error says what to set (--python / --route-cli / FASTROUTE_KICAD_PYTHON / FASTROUTE_ROUTE_CLI).",
};

/// The one-line summary of `--json`.
#[derive(Serialize)]
struct Summary {
    command: &'static str,
    fastroute_version: &'static str,
    /// `complete`, `partial` or `failed`.
    status: &'static str,
    exit_code: u8,
    input: String,
    output: Option<String>,
    unrouted: Option<u64>,
    violations: Option<u64>,
    seconds: f64,
    error: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Flow {
    Specctra,
    Kicad,
}

/// Which flow an input belongs to: by extension, else by content.
fn flow_of(input: &Path) -> Result<Flow, CmdError> {
    let ext = input.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
    match ext.as_deref() {
        Some("dsn") => return Ok(Flow::Specctra),
        Some("kicad_pcb") => return Ok(Flow::Kicad),
        _ => {}
    }
    let head = std::fs::read(input).map_err(|e| CmdError::input(format!("{}: {e}", input.display())))?;
    let head = String::from_utf8_lossy(&head[..head.len().min(64)]).trim_start_matches('\u{feff}').trim_start().to_string();
    if head.starts_with("(kicad_pcb") {
        Ok(Flow::Kicad)
    } else if head.starts_with("(pcb") {
        Ok(Flow::Specctra)
    } else {
        Err(CmdError::input(format!("{}: neither a Specctra design (.dsn) nor a KiCad board (.kicad_pcb)", input.display())))
    }
}

fn default_output(input: &Path, flow: Flow) -> PathBuf {
    let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "board".into());
    input.with_file_name(format!("{stem}.routed.{}", if flow == Flow::Specctra { "ses" } else { "kicad_pcb" }))
}

pub fn run(args: &[String]) -> Result<(), CmdError> {
    let p = SPEC.parse(args)?;
    if p.help {
        print!("{}", SPEC.help_text());
        return Ok(());
    }
    let input: PathBuf = match (p.path("input"), p.positional.as_slice()) {
        (Some(i), []) => i,
        (None, [one]) => PathBuf::from(one),
        (Some(_), _) => return Err(CmdError::usage(format!("unexpected argument '{}': the input is given with --input", p.positional[0]))),
        (None, []) => return Err(CmdError::usage("no input (--input board.dsn or board.kicad_pcb)")),
        (None, _) => return Err(CmdError::usage("more than one input given")),
    };
    let json = p.flag("json");
    let started = Instant::now();
    let result = route(&p, &input);
    let (status, code, unrouted, violations, output, error) = match &result {
        Ok(done) => {
            let partial = done.unrouted.is_some_and(|u| u > 0);
            let code = if partial && !p.flag("allow-partial") { exit::PARTIAL } else { 0 };
            (if partial { "partial" } else { "complete" }, code, done.unrouted, done.violations, Some(done.output.display().to_string()), None)
        }
        Err(e) => ("failed", e.code, None, None, None, Some(e.message.clone())),
    };
    if json {
        let s = Summary {
            command: "route",
            fastroute_version: env!("CARGO_PKG_VERSION"),
            status,
            exit_code: code,
            input: input.display().to_string(),
            output,
            unrouted,
            violations,
            seconds: (started.elapsed().as_secs_f64() * 1000.0).round() / 1000.0,
            error,
        };
        println!("{}", serde_json::to_string(&s).expect("a summary serialises"));
    } else if let Ok(done) = &result {
        if !p.flag("quiet") {
            eprintln!(
                "{}: {} unrouted connections, {} clearance violations -> {}",
                if status == "complete" { "routed completely" } else { "routing finished, connections remain" },
                done.unrouted.map_or("?".to_string(), |u| u.to_string()),
                done.violations.map_or("?".to_string(), |u| u.to_string()),
                done.output.display()
            );
        }
    }
    // (with --json the summary is on stdout and the human message still goes to stderr)
    match (result, code) {
        (Err(e), _) => Err(e),
        (Ok(_), 0) => Ok(()),
        (Ok(_), c) => Err(CmdError::new(c, "the result is written, but connections remain unrouted (exit code 5; --allow-partial makes this 0)")),
    }
}

struct Done {
    output: PathBuf,
    unrouted: Option<u64>,
    violations: Option<u64>,
}

/// Validation runs in the order usage (2), input (3), output (4): the first thing that is wrong
/// with the command line is reported before a problem with the files, and an unusable input
/// before the state of the output.
fn route(p: &Parsed, input: &Path) -> Result<Done, CmdError> {
    let flow = flow_of(input)?;
    let output = p.path("output").unwrap_or_else(|| default_output(input, flow));
    let force = p.flag("force");
    // -- usage
    if let Some(m) = p.value("mode") {
        if !["fast", "exact", "quick"].contains(&m) {
            return Err(CmdError::usage(format!("--mode: '{m}' is not fast, exact or quick")));
        }
    }
    for k in ["max-passes", "threads", "multi-start"] {
        if let Some(v) = p.value(k) {
            v.trim().parse::<u32>().map_err(|_| CmdError::usage(format!("--{k}: '{v}' is not a whole number")))?;
        }
    }
    p.f64("max-time")?;
    p.f64("pair-skew")?;
    if p.flag("verbose") && p.flag("quiet") {
        return Err(CmdError::usage("--verbose and --quiet contradict each other"));
    }
    match flow {
        Flow::Specctra => validate_specctra_usage(p)?,
        Flow::Kicad => validate_kicad_usage(p)?,
    }
    // -- input
    match flow {
        Flow::Specctra => validate_specctra_input(p, input)?,
        Flow::Kicad => validate_kicad_input(input)?,
    }
    // -- output: before the long run starts, not after it
    let init = p.path("initial-session");
    let mut inputs: Vec<&Path> = vec![input];
    inputs.extend(init.as_deref());
    check_output(&output, &inputs, force)?;
    if let Some(r) = p.path("report") {
        let mut all: Vec<&Path> = inputs.clone();
        all.push(&output);
        check_output(&r, &all, force)?;
    }
    match flow {
        Flow::Specctra => route_specctra(p, input, &output),
        Flow::Kicad => route_kicad(p, input, &output),
    }
}

fn validate_specctra_usage(p: &Parsed) -> Result<(), CmdError> {
    kicad_only(p, "Specctra design")?;
    if p.has("diagnose") && !p.has("report") {
        return Err(CmdError::usage("--diagnose needs --report"));
    }
    if p.has("live-port") && !p.flag("live") {
        return Err(CmdError::usage("--live-port needs --live"));
    }
    Ok(())
}

fn validate_kicad_usage(p: &Parsed) -> Result<(), CmdError> {
    for k in ["report", "diagnose", "initial-session", "pairs", "tune", "pair-skew", "layer-heights", "no-neckdown-classes", "parity", "no-enhancements", "live-port"] {
        if p.has(k) {
            return Err(CmdError::usage(format!("--{k} only applies to a Specctra design (.dsn), not to a KiCad board; route_cli.py can pass further fastroute options after `--` (docs/CLI.md)")));
        }
    }
    if p.flag("clear") && p.flag("keep-existing") {
        return Err(CmdError::usage("--clear and --keep-existing contradict each other"));
    }
    Ok(())
}

/// The design must be one the engine can load; an unreadable design is an input error, not a
/// failure of the router.
fn validate_specctra_input(p: &Parsed, input: &Path) -> Result<(), CmdError> {
    let data = std::fs::read(input).map_err(|e| CmdError::input(format!("{}: {e}", input.display())))?;
    load::check_is_dsn(input, &data)?;
    fr_io::load_bytes(&data).map_err(|e| CmdError::input(format!("{}: {e}", input.display())))?;
    if let Some(s) = p.path("initial-session") {
        let bytes = std::fs::read(&s).map_err(|e| CmdError::input(format!("{}: {e}", s.display())))?;
        load::check_is_ses(&s, &bytes)?;
    }
    Ok(())
}

fn validate_kicad_input(input: &Path) -> Result<(), CmdError> {
    let data = std::fs::read(input).map_err(|e| CmdError::input(format!("{}: {e}", input.display())))?;
    if data.is_empty() {
        return Err(CmdError::input(format!("{}: the file is empty", input.display())));
    }
    Ok(())
}

fn kicad_only(p: &Parsed, flow: &str) -> Result<(), CmdError> {
    for k in ["clear", "keep-existing", "python", "route-cli", "work-dir"] {
        if p.has(k) {
            return Err(CmdError::usage(format!("--{k} only applies to a KiCad board (.kicad_pcb), not to a {flow}")));
        }
    }
    Ok(())
}

fn route_specctra(p: &Parsed, input: &Path, output: &Path) -> Result<Done, CmdError> {
    // the legacy flags
    let mut a: Vec<String> = vec!["-de".into(), input.display().to_string(), "-do".into(), output.display().to_string()];
    match p.value("mode") {
        Some("exact") => a.push("--parity".into()),
        Some("quick") => a.push("--router.optimizer.enabled=false".into()),
        _ => {}
    }
    let mut kv = |flag: &str, key: &str| {
        if let Some(v) = p.value(key) {
            a.push(format!("{flag}{v}"));
        }
    };
    kv("--max-time=", "max-time");
    kv("--report=", "report");
    kv("--initial-session=", "initial-session");
    kv("--pairs=", "pairs");
    kv("--tune=", "tune");
    kv("--pair-skew=", "pair-skew");
    kv("--layer-heights=", "layer-heights");
    kv("--no-neckdown-classes=", "no-neckdown-classes");
    kv("--multi-start=", "multi-start");
    kv("--router.optimizer.max_threads=", "threads");
    if let Some(v) = p.value("max-passes") {
        a.push("-mp".into());
        a.push(v.to_string());
    }
    for (flag, key) in [("--diagnose", "diagnose"), ("--parity", "parity"), ("--no-enhancements", "no-enhancements"), ("-v", "verbose"), ("--quiet", "quiet")] {
        if p.flag(key) {
            a.push(flag.into());
        }
    }
    if p.flag("live") {
        a.push(match p.value("live-port") {
            Some(port) => format!("--live={port}"),
            None => "--live".into(),
        });
    } else if p.has("live-port") {
        return Err(CmdError::usage("--live-port needs --live"));
    }
    a.extend(p.passthrough.iter().cloned());
    match crate::run(a) {
        Ok(o) => Ok(Done { output: output.to_path_buf(), unrouted: Some(o.unrouted as u64), violations: Some(o.violations as u64) }),
        Err(e) => Err(CmdError::failed(e)),
    }
}

// ---- KiCad -------------------------------------------------------------------------------

/// Pythons to try, in order, with how each was found (for the error message).
fn python_candidates(p: &Parsed) -> Vec<(String, PathBuf)> {
    let mut v: Vec<(String, PathBuf)> = Vec::new();
    if let Some(x) = p.path("python") {
        v.push(("--python".into(), x));
        return v;
    }
    if let Some(x) = std::env::var_os("FASTROUTE_KICAD_PYTHON").filter(|x| !x.is_empty()) {
        v.push(("$FASTROUTE_KICAD_PYTHON".into(), PathBuf::from(x)));
        return v;
    }
    for x in [
        "/Applications/KiCad/KiCad.app/Contents/Frameworks/Python.framework/Versions/Current/bin/python3",
        "C:\\Program Files\\KiCad\\9.0\\bin\\python.exe",
        "C:\\Program Files\\KiCad\\8.0\\bin\\python.exe",
        "C:\\Program Files\\KiCad\\7.0\\bin\\python.exe",
    ] {
        let x = PathBuf::from(x);
        if x.is_file() {
            v.push(("KiCad's own Python".into(), x));
        }
    }
    v.push(("python3 on PATH".into(), PathBuf::from("python3")));
    v
}

/// What a session import needs from `pcbnew` (the plugin's `core.import_ses` call): in KiCad 7's
/// Python API `ImportSpecctraSES` takes only a file name and works on the board open in the PCB
/// editor, so it raises `TypeError` for the `(board, file)` form that headless use needs (KiCad 8+).
/// Exit code 3 is the probe's "cannot", everything else lets the route go on.
const SESSION_IMPORT_PROBE: &str = "import sys, pcbnew\n\
try:\n    pcbnew.ImportSpecctraSES(pcbnew.BOARD(), '/nonexistent.ses')\n\
except TypeError:\n    print(pcbnew.GetBuildVersion()); sys.exit(3)\n\
except Exception:\n    pass\n";

fn can_import_pcbnew(python: &Path) -> Result<(), String> {
    let out = Command::new(python).args(["-c", "import pcbnew"]).stdin(Stdio::null()).output().map_err(|e| format!("cannot run it: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("import pcbnew failed").trim().to_string());
    }
    let probe = Command::new(python).args(["-c", SESSION_IMPORT_PROBE]).stdin(Stdio::null()).output().map_err(|e| format!("cannot run it: {e}"))?;
    if probe.status.code() == Some(3) {
        let version = String::from_utf8_lossy(&probe.stdout).trim().to_string();
        return Err(format!(
            "KiCad {version}: its Python API cannot import a Specctra session without the PCB editor (ImportSpecctraSES takes only a file name there); routing a board headless needs KiCad 8 or newer"
        ));
    }
    Ok(())
}

fn find_route_cli(p: &Parsed) -> Result<PathBuf, CmdError> {
    if let Some(x) = p.path("route-cli") {
        return if x.is_file() { Ok(x) } else { Err(CmdError::input(format!("{}: no such file (--route-cli)", x.display()))) };
    }
    if let Some(x) = std::env::var_os("FASTROUTE_ROUTE_CLI").filter(|x| !x.is_empty()).map(PathBuf::from) {
        return if x.is_file() { Ok(x) } else { Err(CmdError::input(format!("{}: no such file (FASTROUTE_ROUTE_CLI)", x.display()))) };
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let mut tried = Vec::new();
    // a source checkout (target/<profile>/fastroute), a package (plugins/bin/<platform>/fastroute),
    // or a copy of the plugin next to the program
    for rel in ["../../integrations/kicad/plugins/route_cli.py", "../integrations/kicad/plugins/route_cli.py", "../../route_cli.py", "../route_cli.py", "route_cli.py"] {
        let c = exe.parent().map(|d| d.join(rel)).unwrap_or_default();
        if c.is_file() {
            return Ok(c);
        }
        tried.push(c.display().to_string());
    }
    Err(CmdError::input(format!(
        "route_cli.py of the KiCad plugin was not found (looked at: {}). Give its path with --route-cli or $FASTROUTE_ROUTE_CLI; it is in integrations/kicad/plugins/ of the fastroute sources",
        tried.join(", ")
    )))
}

fn route_kicad(p: &Parsed, input: &Path, output: &Path) -> Result<Done, CmdError> {
    let script = find_route_cli(p)?;
    // the Python with KiCad's module: the first candidate that really imports it
    let mut failures = Vec::new();
    let mut python = None;
    for (how, cand) in python_candidates(p) {
        match can_import_pcbnew(&cand) {
            Ok(()) => {
                python = Some(cand);
                break;
            }
            Err(why) => failures.push(format!("{} ({how}): {why}", cand.display())),
        }
    }
    let Some(python) = python else {
        return Err(CmdError::input(format!(
            "no Python with KiCad's pcbnew module: {}. Routing a .kicad_pcb needs KiCad's Python (KiCad 8 or newer, which can import a session headless): give it with --python or $FASTROUTE_KICAD_PYTHON (macOS: <KiCad.app>/Contents/Frameworks/Python.framework/Versions/Current/bin/python3, Windows: C:\\Program Files\\KiCad\\<version>\\bin\\python.exe). A .dsn file needs no Python: export one from KiCad and route that",
            failures.join("; ")
        )));
    };
    let exe = std::env::current_exe().map_err(|e| CmdError::failed(format!("cannot find this program: {e}")))?;
    let mut a: Vec<String> = vec![script.display().to_string(), input.display().to_string(), "-o".into(), output.display().to_string(), "--fastroute".into(), exe.display().to_string(), "--json".into()];
    if p.flag("force") {
        a.push("--force".into());
    }
    if let Some(m) = p.value("mode") {
        a.extend(["--mode".into(), m.into()]);
    }
    for (flag, key) in [("--max-passes", "max-passes"), ("--threads", "threads"), ("--max-time", "max-time"), ("--work-dir", "work-dir")] {
        if let Some(v) = p.value(key) {
            a.extend([flag.into(), v.into()]);
        }
    }
    for (flag, key) in [("--clear", "clear"), ("--keep-existing", "keep-existing"), ("--live", "live"), ("-q", "quiet")] {
        if p.flag(key) {
            a.push(flag.into());
        }
    }
    if !p.flag("allow-partial") {
        a.push("--fail-on-partial".into());
    }
    if !p.passthrough.is_empty() {
        a.push("--".into());
        a.extend(p.passthrough.iter().cloned());
    }
    let out = Command::new(&python).args(&a).stdin(Stdio::null()).stderr(Stdio::inherit()).output().map_err(|e| CmdError::failed(format!("cannot run {}: {e}", python.display())))?;
    let text = String::from_utf8_lossy(&out.stdout);
    let summary: Option<serde_json::Value> = text.lines().rev().find_map(|l| serde_json::from_str(l.trim()).ok());
    let num = |k: &str| summary.as_ref().and_then(|s| s.get(k)).and_then(|v| v.as_u64());
    let message = summary.as_ref().and_then(|s| s.get("error")).and_then(|v| v.as_str()).map(str::to_string);
    match out.status.code() {
        Some(0) | Some(5) => {
            if summary.is_none() {
                return Err(CmdError::failed("the KiCad adapter finished but printed no summary (is route_cli.py an old version without --json?)"));
            }
            Ok(Done { output: output.to_path_buf(), unrouted: num("unrouted"), violations: num("violations") })
        }
        Some(4) => Err(CmdError::output(message.unwrap_or_else(|| "the KiCad adapter refused the output".into()))),
        Some(2) => Err(CmdError::input(message.unwrap_or_else(|| "the KiCad adapter rejected the board".into()))),
        other => Err(CmdError::failed(message.unwrap_or_else(|| format!("the KiCad adapter failed ({})", other.map(|c| format!("exit code {c}")).unwrap_or_else(|| "killed by a signal".into()))))),
    }
}
