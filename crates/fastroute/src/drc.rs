//! KiCad's design rule check as a data source: reading a `kicad-cli pcb drc --format json`
//! report, and running `kicad-cli` to make one.
//!
//! The rule that matters: a DRC result exists only if a real checker produced it. Not run,
//! missing tool, timeout, crash and unreadable output are separate statuses, and **none of them
//! is ever reported as "passed"**. `passed` needs a readable report with no violations and no
//! unconnected items.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DrcStatus {
    /// Nobody asked for a DRC (or the input for it was not given).
    NotRun,
    /// `kicad-cli` was not found.
    Unavailable,
    /// The tool failed or its output could not be read.
    Error,
    /// The tool did not finish in time and was stopped.
    Timeout,
    /// A report exists and lists violations or unconnected items.
    Failed,
    /// A report exists and lists none.
    Passed,
}

impl DrcStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            DrcStatus::NotRun => "not_run",
            DrcStatus::Unavailable => "unavailable",
            DrcStatus::Error => "error",
            DrcStatus::Timeout => "timeout",
            DrcStatus::Failed => "failed",
            DrcStatus::Passed => "passed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrcIssue {
    /// `DRC1`, `DRC2`, ... (violations first, then unconnected items).
    pub id: String,
    /// `violation` or `unconnected`.
    pub kind: String,
    /// KiCad's rule type, e.g. `clearance`, `unconnected_items`.
    #[serde(rename = "type")]
    pub rule: String,
    pub severity: String,
    pub description: String,
    pub x: f64,
    pub y: f64,
    /// KiCad's descriptions of the involved items.
    pub items: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrcSummary {
    pub kicad_version: Option<String>,
    pub violations: u32,
    pub unconnected: u32,
    pub errors: u32,
    pub warnings: u32,
    pub by_type: BTreeMap<String, u32>,
    pub issues: Vec<DrcIssue>,
}

fn pos(v: &Value) -> Option<(f64, f64)> {
    Some((v.get("x")?.as_f64()?, v.get("y")?.as_f64()?))
}

/// Reads a KiCad DRC JSON report. Errors say what is wrong with the document.
pub fn summarize(text: &str) -> Result<DrcSummary, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    if !v.is_object() || !(v.get("violations").is_some_and(Value::is_array) || v.get("unconnected_items").is_some_and(Value::is_array)) {
        return Err("not a KiCad DRC report (no \"violations\" / \"unconnected_items\" lists)".into());
    }
    if let Some(u) = v.get("coordinate_units").and_then(Value::as_str) {
        if u != "mm" {
            return Err(format!("the report uses \"{u}\" as coordinate unit; fastroute reads mm (run kicad-cli with --units mm)"));
        }
    }
    let mut s = DrcSummary {
        kicad_version: v.get("kicad_version").and_then(Value::as_str).map(str::to_string),
        violations: 0,
        unconnected: 0,
        errors: 0,
        warnings: 0,
        by_type: BTreeMap::new(),
        issues: Vec::new(),
    };
    for (key, kind) in [("violations", "violation"), ("unconnected_items", "unconnected")] {
        for (i, x) in v[key].as_array().into_iter().flatten().enumerate() {
            let pts: Vec<(f64, f64)> = x["items"].as_array().into_iter().flatten().filter_map(|it| pos(&it["pos"])).collect();
            if pts.is_empty() {
                return Err(format!("{key}[{i}] has no item with a position"));
            }
            // the marker sits in the middle of the box around the involved items
            let (minx, maxx) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            let (miny, maxy) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            let severity = x.get("severity").and_then(Value::as_str).unwrap_or("error").to_string();
            let rule = x.get("type").and_then(Value::as_str).unwrap_or(if kind == "unconnected" { "unconnected_items" } else { "unknown" }).to_string();
            match severity.as_str() {
                "error" => s.errors += 1,
                "warning" => s.warnings += 1,
                _ => {}
            }
            if kind == "violation" {
                s.violations += 1;
            } else {
                s.unconnected += 1;
            }
            *s.by_type.entry(rule.clone()).or_insert(0) += 1;
            s.issues.push(DrcIssue {
                id: format!("DRC{}", s.issues.len() + 1),
                kind: kind.into(),
                rule,
                severity,
                description: x.get("description").and_then(Value::as_str).unwrap_or("").to_string(),
                x: (minx + maxx) / 2.0,
                y: (miny + maxy) / 2.0,
                items: x["items"].as_array().into_iter().flatten().filter_map(|it| it.get("description").and_then(Value::as_str)).map(str::to_string).collect(),
            });
        }
    }
    Ok(s)
}

impl DrcSummary {
    pub fn status(&self) -> DrcStatus {
        if self.issues.is_empty() {
            DrcStatus::Passed
        } else {
            DrcStatus::Failed
        }
    }
}

/// What a DRC attempt came to.
#[derive(Clone, Debug, PartialEq)]
pub struct DrcOutcome {
    pub status: DrcStatus,
    /// The tool that produced the report (`kicad-cli` and its path), if one ran.
    pub tool: Option<String>,
    pub tool_version: Option<String>,
    pub detail: Option<String>,
    /// The report as written by the tool.
    pub report_text: Option<String>,
    pub summary: Option<DrcSummary>,
}

impl DrcOutcome {
    pub fn not_run(detail: impl Into<String>) -> DrcOutcome {
        DrcOutcome { status: DrcStatus::NotRun, tool: None, tool_version: None, detail: Some(detail.into()), report_text: None, summary: None }
    }
    fn failure(status: DrcStatus, tool: &Path, version: Option<String>, detail: impl Into<String>) -> DrcOutcome {
        DrcOutcome { status, tool: Some(tool.display().to_string()), tool_version: version, detail: Some(detail.into()), report_text: None, summary: None }
    }
    /// An existing report (`--drc-report`): the status comes from its content alone.
    pub fn from_report_text(text: String, source: &str) -> DrcOutcome {
        match summarize(&text) {
            Ok(s) => DrcOutcome {
                status: s.status(),
                tool: None,
                tool_version: s.kicad_version.clone(),
                detail: Some(format!("read from {source}; fastroute did not run the check")),
                report_text: Some(text),
                summary: Some(s),
            },
            Err(e) => DrcOutcome { status: DrcStatus::Error, tool: None, tool_version: None, detail: Some(format!("{source}: {e}")), report_text: None, summary: None },
        }
    }
}

/// Where `kicad-cli` is: the configured path, `$FASTROUTE_KICAD_CLI`, then `PATH`, then the
/// usual install locations of KiCad.
pub fn find_kicad_cli(configured: Option<&Path>) -> Result<PathBuf, String> {
    let exe_names: &[&str] = if cfg!(windows) { &["kicad-cli.exe"] } else { &["kicad-cli"] };
    if let Some(p) = configured {
        return if p.is_file() { Ok(p.to_path_buf()) } else { Err(format!("{}: no such file (--kicad-cli)", p.display())) };
    }
    if let Some(p) = std::env::var_os("FASTROUTE_KICAD_CLI").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) {
        return if p.is_file() { Ok(p) } else { Err(format!("{}: no such file (FASTROUTE_KICAD_CLI)", p.display())) };
    }
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/Applications/KiCad/KiCad.app/Contents/MacOS", "/usr/local/bin", "/usr/bin", "C:\\Program Files\\KiCad\\9.0\\bin", "C:\\Program Files\\KiCad\\8.0\\bin"].map(PathBuf::from));
    for d in dirs {
        for n in exe_names {
            let c = d.join(n);
            if c.is_file() {
                return Ok(c);
            }
        }
    }
    Err("kicad-cli was not found (install KiCad 7 or newer, put kicad-cli on PATH, or use --kicad-cli / FASTROUTE_KICAD_CLI)".into())
}

struct Finished {
    code: Option<i32>,
    out: String,
    err: String,
    timed_out: bool,
}

/// Runs `cmd` for at most `timeout`; the child is killed when the time is up.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> std::io::Result<Finished> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let drain = |s: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            if let Some(mut r) = s {
                let mut bytes = Vec::new();
                let _ = r.read_to_end(&mut bytes);
                text = String::from_utf8_lossy(&bytes).into_owned();
            }
            text
        })
    };
    let out = drain(child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>));
    let start = Instant::now();
    let (code, timed_out) = loop {
        if let Some(status) = child.try_wait()? {
            break (status.code(), false);
        }
        if start.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Finished { code, out: out.join().unwrap_or_default(), err: err.join().unwrap_or_default(), timed_out })
}

fn first_line(s: &str) -> String {
    s.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").chars().take(300).collect()
}

/// Runs `kicad-cli pcb drc` on `pcb` and reads the report it writes to `report_path`.
pub fn run_kicad_cli(tool: &Path, pcb: &Path, report_path: &Path, timeout: Duration) -> DrcOutcome {
    let version = {
        let mut c = Command::new(tool);
        c.arg("--version");
        run_with_timeout(c, Duration::from_secs(10)).ok().filter(|f| f.code == Some(0)).map(|f| first_line(&f.out)).filter(|v| !v.is_empty())
    };
    let _ = std::fs::remove_file(report_path);
    let mut c = Command::new(tool);
    c.args(["pcb", "drc", "--format", "json", "--units", "mm", "--severity-all", "--output"]).arg(report_path).arg(pcb);
    let fin = match run_with_timeout(c, timeout) {
        Ok(f) => f,
        Err(e) => return DrcOutcome::failure(DrcStatus::Error, tool, version, format!("cannot run {}: {e}", tool.display())),
    };
    if fin.timed_out {
        let _ = std::fs::remove_file(report_path);
        return DrcOutcome::failure(DrcStatus::Timeout, tool, version, format!("kicad-cli did not finish within {} s and was stopped", timeout.as_secs()));
    }
    if fin.code != Some(0) {
        let why = first_line(&fin.err).to_string();
        let why = if why.is_empty() { first_line(&fin.out) } else { why };
        return DrcOutcome::failure(DrcStatus::Error, tool, version, format!("kicad-cli exited with {}{}", fin.code.map(|c| format!("code {c}")).unwrap_or_else(|| "a signal".into()), if why.is_empty() { String::new() } else { format!(": {why}") }));
    }
    let text = match std::fs::read_to_string(report_path) {
        Ok(t) => t,
        Err(e) => return DrcOutcome::failure(DrcStatus::Error, tool, version, format!("kicad-cli finished but wrote no readable report at {}: {e}", report_path.display())),
    };
    match summarize(&text) {
        Ok(s) => DrcOutcome { status: s.status(), tool: Some(tool.display().to_string()), tool_version: version.or_else(|| s.kicad_version.clone()), detail: None, report_text: Some(text), summary: Some(s) },
        Err(e) => DrcOutcome::failure(DrcStatus::Error, tool, version, format!("the report of kicad-cli cannot be read: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPORT: &str = r#"{"$schema":"https://schemas.kicad.org/drc.v1.json","kicad_version":"9.0.1","coordinate_units":"mm",
        "violations":[
          {"type":"clearance","severity":"error","description":"Clearance violation","items":[{"description":"Track [A]","pos":{"x":10,"y":6}},{"description":"Track [B]","pos":{"x":12,"y":6.5}}]},
          {"type":"silk_overlap","severity":"warning","description":"Silkscreen overlap","items":[{"description":"Text","pos":{"x":1,"y":1}}]}],
        "unconnected_items":[{"type":"unconnected_items","severity":"error","description":"Missing connection","items":[{"description":"Pad 1","pos":{"x":30,"y":18}},{"description":"Pad 2","pos":{"x":30,"y":21}}]}],
        "schematic_parity":[]}"#;

    #[test]
    fn summary_counts_and_positions() {
        let s = summarize(REPORT).unwrap();
        assert_eq!((s.violations, s.unconnected, s.errors, s.warnings), (2, 1, 2, 1));
        assert_eq!(s.kicad_version.as_deref(), Some("9.0.1"));
        assert_eq!(s.by_type["clearance"], 1);
        assert_eq!(s.status(), DrcStatus::Failed);
        assert_eq!(s.issues.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["DRC1", "DRC2", "DRC3"]);
        assert_eq!((s.issues[0].x, s.issues[0].y), (11.0, 6.25));
        assert_eq!(s.issues[2].kind, "unconnected");
        assert_eq!(s.issues[0].items, ["Track [A]", "Track [B]"]);
    }

    #[test]
    fn only_a_clean_readable_report_is_passed() {
        let clean = r#"{"kicad_version":"9.0.1","coordinate_units":"mm","violations":[],"unconnected_items":[],"schematic_parity":[]}"#;
        assert_eq!(summarize(clean).unwrap().status(), DrcStatus::Passed);
        for bad in ["", "{}", "[]", "null", r#"{"violations":"none"}"#, r#"{"violations":[{"items":[]}]}"#, r#"{"coordinate_units":"inch","violations":[]}"#] {
            assert!(summarize(bad).is_err(), "{bad:?} must not read as a report");
        }
        let o = DrcOutcome::from_report_text("{}".into(), "x.json");
        assert_eq!(o.status, DrcStatus::Error);
        assert!(o.summary.is_none());
    }

    #[test]
    fn missing_tool_is_reported_not_guessed() {
        let e = find_kicad_cli(Some(Path::new("/no/such/kicad-cli"))).unwrap_err();
        assert!(e.contains("/no/such/kicad-cli"));
    }
}
