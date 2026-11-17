//! `fastroute ai-pack`: one command that writes a local directory to hand to an AI model (or a
//! colleague): the whole board, a region as a combined and per-layer pictures, the board as
//! JSON, the design rules, the routing report and the result of KiCad's DRC, tied together by
//! `manifest.json`. Everything stays on this machine; nothing is uploaded and the input files
//! are only read.
//!
//! ```text
//! overview.png            the whole board
//! region-combined.png     the region of interest, all selected layers
//! region-<layer>.png      the same region, one copper layer (only layers with copper in it)
//! inspect.json            the board as data (fastroute inspect)
//! rules.json              the design rules (or the file given with --rules)
//! route-report.json       fastroute's --report summary (or the file given with --report)
//! drc-report.json         KiCad's own DRC JSON, only if a real DRC ran / was supplied
//! manifest.json           images, regions, transforms, reports and their status; written last
//! ```
//!
//! The DRC is never invented: without `kicad-cli` (or a board to run it on) the pack says
//! `unavailable` / `not_run`, warns, and carries on; `drc-report.json` then does not exist.
//! See `docs/AI_DATA_FORMAT.md`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use fr_render::{Annotations, BoardGeometry, Rect, RegionSpec, SizeSpec, View};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cmd::{exit, opt, same_file, write_atomically, CmdError, Opt, Spec};
use crate::drc::{self, DrcOutcome, DrcStatus};
use crate::extract;
use crate::inspect;
use crate::load::{self, FileInfo};
use crate::manifest::{ErrorEntry, Manifest, ReportEntry, Selection};
use crate::render_cmd::{self, Request};
use crate::reports;

pub const OVERVIEW: &str = "overview.png";
pub const REGION_COMBINED: &str = "region-combined.png";
pub const INSPECT: &str = "inspect.json";
pub const RULES: &str = "rules.json";
pub const ROUTE_REPORT: &str = "route-report.json";
pub const DRC_REPORT: &str = "drc-report.json";
pub const MANIFEST: &str = "manifest.json";

/// Pictures of a tiny region are not blown up beyond this many pixels per mm by default.
const MAX_DEFAULT_PX_PER_MM: f64 = 150.0;

const OPTS: &[Opt] = &[
    opt("input", Some('i'), Some("FILE"), "Specctra design (.dsn) (also the first positional argument)"),
    opt("session", Some('s'), Some("FILE"), "session (.ses) with the wiring to analyse, e.g. the result of a route run"),
    opt("output", Some('o'), Some("DIR"), "directory to write the pack into (created; an existing non-empty one needs --force)"),
    opt("around", None, Some("REF"), "region of interest: the package of this component plus --padding-mm"),
    opt("region-mm", None, Some("x,y,w,h"), "region of interest: this rectangle (top-left corner, mm, y points down)"),
    opt("net", None, Some("NAME"), "region of interest: everything of this net (repeat for a differential pair)"),
    opt("net-planes", None, None, "with --net: planes of the net count for the region too"),
    opt("from-report", None, Some("FILE"), "region of interest from the problems in a report (fastroute --report, inspect.json or KiCad DRC JSON)"),
    opt("select", None, Some("WHAT"), "which problems: all, unrouted, violations, drc or ids like OPEN1,CLR3"),
    opt("padding-mm", None, Some("N"), "margin around the region of interest (default 2; 3 for problems)"),
    opt("layers", Some('l'), Some("LIST"), "copper layers of region-combined.png and of the per-layer pictures (default all)"),
    opt("size", None, Some("N|WxH"), "size of the region pictures in pixels (default: 2048 on the longer side)"),
    opt("px-per-mm", None, Some("S"), "scale of the region pictures instead of --size (default is at most 150 px/mm)"),
    opt("overview-size", None, Some("N"), "pixels on the longer side of overview.png (default 2048)"),
    opt("annotate", None, Some("LIST"), "labels of the region pictures (default all: grid, nets, pads, refs, scale, layer, markers)"),
    opt("kicad-pcb", None, Some("FILE"), "the routed KiCad board (.kicad_pcb) to run KiCad's DRC on"),
    opt("kicad-cli", None, Some("FILE"), "path of kicad-cli (default: $FASTROUTE_KICAD_CLI, PATH, the usual install places)"),
    opt("drc-timeout", None, Some("SECONDS"), "stop kicad-cli after this time (default 300)"),
    opt("drc-report", None, Some("FILE"), "a DRC report that was made elsewhere (kicad-cli pcb drc --format json --units mm)"),
    opt("no-drc", None, None, "do not run or read a DRC"),
    opt("require-drc", None, None, "exit with code 5 unless a DRC report exists (passed or failed); the pack is written anyway"),
    opt("report", None, Some("FILE"), "the --report JSON of a route run, to include as route-report.json (default: made from the board)"),
    opt("rules", None, Some("FILE"), "your own rules JSON, included as rules.json (default: the rules of the design)"),
    opt("verify", None, Some("DIR"), "do not make a pack: check an existing one (files exist, hashes and statuses agree)"),
    opt("force", None, None, "replace the pack files in an existing directory (nothing else in it is touched)"),
    opt("quiet", Some('q'), None, "no summary line"),
];

pub const SPEC: Spec = Spec {
    name: "ai-pack",
    about: "write a local analysis pack (pictures, data, rules, DRC) for an AI model",
    usage: "ai-pack --input design.dsn [--session routed.ses] --output DIR [options]\n       fastroute ai-pack --verify DIR",
    opts: OPTS,
    passthrough: &[],
    footer: "Without a region option the region of interest is around the problems that were found\n\
             (unrouted connections, clearance violations, DRC); without problems it is the whole board.\n\
             KiCad's DRC needs the routed board: --kicad-pcb routed.kicad_pcb (and kicad-cli of KiCad 8 or newer).\n\
             If that is missing the pack says drc_status \"unavailable\" or \"not_run\" - never \"passed\" -\n\
             warns, and carries on. Nothing is uploaded anywhere. Format: docs/AI_DATA_FORMAT.md.",
};

#[derive(Serialize, Deserialize)]
struct RulesDoc {
    format: String,
    format_version: String,
    fastroute_version: String,
    generated_by: String,
    source: RulesSource,
    units: String,
    layers: Vec<String>,
    net_classes: Vec<inspect::ClassOut>,
    note: String,
}

#[derive(Serialize, Deserialize)]
struct RulesSource {
    kind: String,
    file: String,
    sha256: String,
}

fn sanitize(layer: &str) -> String {
    let s: String = layer.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' }).collect();
    if s.is_empty() {
        "layer".into()
    } else {
        s
    }
}

/// The names the pack owns in its directory.
fn is_pack_file(name: &str) -> bool {
    [OVERVIEW, REGION_COMBINED, INSPECT, RULES, ROUTE_REPORT, DRC_REPORT, MANIFEST].contains(&name) || (name.starts_with("region-") && name.ends_with(".png"))
}

fn layer_has_content(g: &BoardGeometry, layer: usize, region: &Rect) -> bool {
    g.pads.iter().any(|p| p.layer == layer && p.shape.bbox().is_some_and(|b| b.intersects(region)))
        || g.traces.iter().any(|t| t.layer == layer && t.bbox().is_some_and(|b| b.intersects(region)))
        || g.vias.iter().any(|v| v.spans(layer) && v.bbox().intersects(region))
        || g.planes.iter().any(|a| a.layer == layer && Rect::from_points(&a.pts).is_some_and(|b| b.intersects(region)))
}

fn drc_report_status(s: DrcStatus) -> &'static str {
    match s {
        DrcStatus::Passed | DrcStatus::Failed => "present",
        DrcStatus::NotRun => "not_run",
        DrcStatus::Unavailable => "unavailable",
        DrcStatus::Error => "failed",
        DrcStatus::Timeout => "timeout",
    }
}

fn read_json_file(path: &Path, what: &str) -> Result<String, CmdError> {
    let text = std::fs::read_to_string(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    serde_json::from_str::<Value>(&text).map_err(|e| CmdError::input(format!("{}: {what} is not valid JSON: {e}", path.display())))?;
    Ok(text)
}

pub fn run(args: &[String]) -> Result<(), CmdError> {
    let p = SPEC.parse(args)?;
    if p.help {
        print!("{}", SPEC.help_text());
        return Ok(());
    }
    if let Some(dir) = p.path("verify") {
        if p.has("input") || p.has("output") {
            return Err(CmdError::usage("--verify checks an existing pack; it cannot be combined with --input / --output"));
        }
        let problems = verify_pack(&dir);
        return if problems.is_empty() {
            if !p.flag("quiet") {
                println!("{}: the pack is consistent", dir.display());
            }
            Ok(())
        } else {
            Err(CmdError::input(format!("{}: {} problem(s):\n  {}", dir.display(), problems.len(), problems.join("\n  "))))
        };
    }

    let input: PathBuf = match (p.path("input"), p.positional.as_slice()) {
        (Some(i), []) => i,
        (None, [one]) => PathBuf::from(one),
        (Some(_), _) => return Err(CmdError::usage(format!("unexpected argument '{}': the input is given with --input", p.positional[0]))),
        (None, []) => return Err(CmdError::usage("no input design (--input design.dsn)")),
        (None, _) => return Err(CmdError::usage("more than one input design given")),
    };
    let dir = p.path("output").ok_or_else(|| CmdError::usage("no output directory (--output DIR)"))?;
    let session = p.path("session");
    let force = p.flag("force");
    let mut inputs: Vec<PathBuf> = vec![input.clone()];
    for k in ["session", "kicad-pcb", "drc-report", "report", "rules", "from-report"] {
        inputs.extend(p.path(k));
    }
    // options that only mean something together, and the cheap checks, before any work
    if p.flag("no-drc") && (p.has("kicad-pcb") || p.has("drc-report") || p.flag("require-drc")) {
        return Err(CmdError::usage("--no-drc contradicts --kicad-pcb, --drc-report and --require-drc"));
    }
    if p.has("kicad-pcb") && p.has("drc-report") {
        return Err(CmdError::usage("give either --kicad-pcb (run the DRC) or --drc-report (use a finished one), not both"));
    }
    let timeout = match p.f64("drc-timeout")? {
        Some(t) if t >= 1.0 => Duration::from_secs_f64(t),
        Some(t) => return Err(CmdError::usage(format!("--drc-timeout must be at least 1 second, not {t}"))),
        None => drc::DEFAULT_TIMEOUT,
    };
    let overview_size = match p.value("overview-size") {
        Some(v) => SizeSpec::LongSide(v.trim().parse::<u32>().ok().filter(|n| *n > 0).ok_or_else(|| CmdError::usage(format!("--overview-size: '{v}' is not a number of pixels")))?),
        None => SizeSpec::LongSide(2048),
    };
    let annotations = match p.value("annotate") {
        Some(a) => Annotations::parse(a).map_err(CmdError::usage)?,
        None => Annotations::all(),
    };
    // the pack must not overwrite what it is made from
    for name in [OVERVIEW, REGION_COMBINED, INSPECT, RULES, ROUTE_REPORT, DRC_REPORT, MANIFEST] {
        if let Some(i) = inputs.iter().find(|i| same_file(i, &dir.join(name))) {
            return Err(CmdError::output(format!("{}: an input file would be overwritten by the pack file {name}; give another --output", i.display())));
        }
    }
    check_dir(&dir, force)?;
    let kicad_pcb = p.path("kicad-pcb");
    if let Some(pcb) = &kicad_pcb {
        if !pcb.is_file() {
            return Err(CmdError::input(format!("{}: no such file (--kicad-pcb)", pcb.display())));
        }
    }
    // read what is only read here, before anything in the directory is replaced
    let user_rules = p.path("rules").map(|f| read_json_file(&f, "the rules file").map(|t| (f, t))).transpose()?;
    let given_route_report = p.path("report").map(|f| read_json_file(&f, "the route report").map(|t| (f, t))).transpose()?;
    let given_drc = match p.path("drc-report") {
        Some(f) => {
            let text = std::fs::read_to_string(&f).map_err(|e| CmdError::input(format!("{}: {e}", f.display())))?;
            let o = DrcOutcome::from_report_text(text, &f.display().to_string());
            if o.status == DrcStatus::Error {
                return Err(CmdError::input(o.detail.unwrap_or_default()));
            }
            Some(o)
        }
        None => None,
    };

    let t_start = Instant::now();
    let loaded = match load::load(&input, session.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            // an unreadable input is still reported inside the directory, not only on stderr
            if prepare_dir(&dir, force).is_ok() {
                let file_info = |p: &Path| std::fs::read(p).ok().map(|d| FileInfo::of(p, &d));
                let mut m = Manifest::new("fastroute ai-pack", file_info(&input), session.as_deref().and_then(file_info));
                m.status = "error".into();
                m.errors.push(ErrorEntry { code: inspect::error_code(e.code).into(), message: e.message.clone() });
                let _ = write_atomically(&dir.join(MANIFEST), m.to_json().as_bytes());
            }
            return Err(e);
        }
    };
    let load_s = t_start.elapsed().as_secs_f64();
    let mut analysis = extract::analyse(&loaded.board, &loaded.name, &loaded.synthetic);

    // the directory is replaced only now that everything is read
    prepare_dir(&dir, force)?;
    let mut warnings = loaded.warnings.clone();

    // ---- DRC -----------------------------------------------------------------------------
    let drc_path = dir.join(DRC_REPORT);
    let drc_outcome = if let Some(o) = given_drc {
        o
    } else if p.flag("no-drc") {
        DrcOutcome::not_run("--no-drc")
    } else if let Some(pcb) = &kicad_pcb {
        match drc::find_kicad_cli(p.path("kicad-cli").as_deref()) {
            Ok(tool) => drc::run_kicad_cli(&tool, pcb, &drc_path, timeout),
            Err(why) => DrcOutcome { status: DrcStatus::Unavailable, tool: None, tool_version: None, detail: Some(why), report_text: None, summary: None },
        }
    } else {
        DrcOutcome::not_run("no KiCad board (--kicad-pcb) and no finished report (--drc-report) was given")
    };
    match &drc_outcome.report_text {
        Some(text) => write_atomically(&drc_path, text.as_bytes())?,
        None => {
            let _ = std::fs::remove_file(&drc_path);
        }
    }
    if let Some(text) = &drc_outcome.report_text {
        // the problems KiCad found become markers DRC1.. in the pictures
        let v: Value = serde_json::from_str(text).map_err(|e| CmdError::failed(e.to_string()))?;
        analysis.geometry.issues.extend(reports::kicad_drc_issues(&v).map_err(CmdError::failed)?);
    }
    match drc_outcome.status {
        DrcStatus::Passed | DrcStatus::Failed => {}
        s => warnings.push(format!(
            "DRC: {}. drc_status is \"{}\", which is not a pass; the pack was written without a DRC result",
            drc_outcome.detail.clone().unwrap_or_else(|| "not available".into()),
            s.as_str()
        )),
    }

    // ---- pictures ------------------------------------------------------------------------
    let g = &analysis.geometry;
    let all_layers = g.parse_layers(p.value("layers").unwrap_or("all"))?;
    let mut manifest = Manifest::new("fastroute ai-pack", Some(loaded.input.clone()), loaded.session.clone());
    manifest.drc_status = drc_outcome.status.as_str().into();

    let overview = render_cmd::render_one(
        g,
        &Request {
            region: RegionSpec::Board { margin_mm: None },
            selection: Selection { kind: "board".into(), targets: vec![], padding_mm: None },
            layers: g.all_layers(),
            size: overview_size,
            annotations: Annotations::parse("scale,layer,refs,markers").expect("valid list"),
            airlines: true,
        },
    )?;
    write_image(&dir, OVERVIEW, "overview", &Selection { kind: "board".into(), targets: vec![], padding_mm: None }, &overview, &mut manifest)?;

    // the region of interest: as asked, else around the problems, else the whole board
    let (spec, selection) = if ["region-mm", "around", "net", "from-report", "select"].iter().any(|k| p.has(k)) {
        let mut parsed_ids = None;
        if let Some(f) = p.path("from-report") {
            // problems from a file replace those of the board when they are fastroute's own
            let rep = reports::read_report(&f)?;
            parsed_ids = Some(rep.issues.iter().map(|i| i.id.clone()).collect::<Vec<_>>());
            match rep.kind {
                reports::ReportKind::KicadDrc => analysis.geometry.issues.extend(rep.issues),
                _ => analysis.geometry.issues = rep.issues,
            }
        }
        render_cmd::region_from_args(&p, &analysis.geometry, parsed_ids.as_deref())?
    } else if !analysis.geometry.issues.is_empty() {
        let ids: Vec<String> = analysis.geometry.issues.iter().map(|i| i.id.clone()).collect();
        (RegionSpec::Issues { ids: ids.clone(), padding_mm: 3.0 }, Selection { kind: "issues".into(), targets: ids, padding_mm: Some(3.0) })
    } else {
        (RegionSpec::Board { margin_mm: None }, Selection { kind: "board".into(), targets: vec![], padding_mm: None })
    };
    let g = &analysis.geometry;
    let region: Rect = fr_render::resolve(g, &spec, &all_layers)?;
    let size = match (p.has("size") || p.has("px-per-mm"), render_cmd::parse_size(&p)?) {
        (true, s) => s,
        (false, _) => match View::fit(region, SizeSpec::LongSide(2048)) {
            Ok(v) if v.px_per_mm > MAX_DEFAULT_PX_PER_MM => SizeSpec::PxPerMm(MAX_DEFAULT_PX_PER_MM),
            _ => SizeSpec::LongSide(2048),
        },
    };
    let region_request = |layers: Vec<usize>| Request {
        // (the same rectangle for every picture of the pack)
        region: RegionSpec::Rect(region),
        selection: selection.clone(),
        layers,
        size,
        annotations: annotations.clone(),
        airlines: true,
    };
    let combined = render_cmd::render_one(g, &region_request(all_layers.clone()))?;
    write_image(&dir, REGION_COMBINED, "region-combined", &selection, &combined, &mut manifest)?;
    if g.layers.len() > 1 {
        let mut used: Vec<String> = Vec::new();
        for &l in &all_layers {
            if !layer_has_content(g, l, &combined.meta.region_mm.to_rect()) {
                continue;
            }
            let mut stem = sanitize(&g.layers[l].name);
            while used.contains(&stem) {
                stem.push_str(&format!("-{l}"));
            }
            used.push(stem.clone());
            let r = render_cmd::render_one(g, &region_request(vec![l]))?;
            write_image(&dir, &format!("region-{stem}.png"), "region-layer", &selection, &r, &mut manifest)?;
        }
    }

    // ---- data and reports ----------------------------------------------------------------
    let drc_file = drc_outcome.report_text.is_some().then(|| DRC_REPORT.to_string());
    let doc = inspect::build(&loaded, &analysis, &drc_outcome, drc_file.clone());
    write_atomically(&dir.join(INSPECT), doc.to_json().as_bytes())?;
    manifest.reports.insert("inspect".into(), ReportEntry::of_file(&dir, INSPECT, "present", None));

    let rules_detail = match &user_rules {
        Some((f, text)) => {
            write_atomically(&dir.join(RULES), text.as_bytes())?;
            Some(format!("copied from {}", f.display()))
        }
        None => {
            let doc = RulesDoc {
                format: "fastroute-rules".into(),
                format_version: "1.0".into(),
                fastroute_version: env!("CARGO_PKG_VERSION").into(),
                generated_by: "fastroute ai-pack".into(),
                source: RulesSource { kind: "dsn".into(), file: loaded.input.path.clone(), sha256: loaded.input.sha256.clone() },
                units: "mm".into(),
                layers: g.layers.iter().map(|l| l.name.clone()).collect(),
                net_classes: doc.rules.as_ref().map(|r| r.net_classes.clone()).unwrap_or_default(),
                note: doc.rules.as_ref().map(|r| r.note.clone()).unwrap_or_default(),
            };
            let mut text = serde_json::to_string_pretty(&doc).expect("rules serialise");
            text.push('\n');
            write_atomically(&dir.join(RULES), text.as_bytes())?;
            Some("the net classes of the design".into())
        }
    };
    manifest.reports.insert("rules".into(), ReportEntry::of_file(&dir, RULES, "present", rules_detail));

    let route_detail = match &given_route_report {
        Some((f, text)) => {
            write_atomically(&dir.join(ROUTE_REPORT), text.as_bytes())?;
            format!("copied from {}", f.display())
        }
        None => {
            let timings = crate::report::Timings { load_s, route_s: 0.0, total_s: t_start.elapsed().as_secs_f64() };
            crate::report::write_report(&dir.join(ROUTE_REPORT).to_string_lossy(), &loaded.input.path, &loaded.board, None, &loaded.settings, true, true, &timings)
                .map_err(|e| CmdError::output(format!("{}: {e}", dir.join(ROUTE_REPORT).display())))?;
            "made from the loaded board; no routing was run by ai-pack, so time_s.route is 0 (pass --report to include the report of a route run)".into()
        }
    };
    manifest.reports.insert("route_report".into(), ReportEntry::of_file(&dir, ROUTE_REPORT, "present", Some(route_detail)));

    let drc_detail = drc_outcome.detail.clone().or_else(|| drc_outcome.tool.as_ref().map(|t| format!("made by {t}{}", drc_outcome.tool_version.as_ref().map(|v| format!(" ({v})")).unwrap_or_default())));
    manifest.reports.insert(
        "drc_report".into(),
        match drc_file {
            Some(f) => ReportEntry::of_file(&dir, &f, drc_report_status(drc_outcome.status), drc_detail),
            None => ReportEntry::missing(drc_report_status(drc_outcome.status), drc_detail),
        },
    );
    manifest.warnings = warnings.clone();
    if matches!(drc_outcome.status, DrcStatus::Unavailable | DrcStatus::Error | DrcStatus::Timeout) {
        manifest.status = "partial".into();
    }
    write_atomically(&dir.join(MANIFEST), manifest.to_json().as_bytes())?;

    // the pack must agree with itself before it is handed out
    let problems = verify_pack(&dir);
    if !problems.is_empty() {
        return Err(CmdError::failed(format!("the pack is inconsistent (a bug): {}", problems.join("; "))));
    }
    if !p.flag("quiet") {
        for w in &warnings {
            eprintln!("warning: {w}");
        }
        println!(
            "wrote {}: {} pictures, {}, {}, {}, drc_status {}",
            dir.display(),
            manifest.images.len(),
            INSPECT,
            RULES,
            ROUTE_REPORT,
            manifest.drc_status
        );
    }
    if p.flag("require-drc") && !matches!(drc_outcome.status, DrcStatus::Passed | DrcStatus::Failed) {
        return Err(CmdError::new(exit::PARTIAL, format!("the pack is written, but there is no DRC result (drc_status \"{}\")", drc_outcome.status.as_str())));
    }
    Ok(())
}

fn write_image(dir: &Path, name: &str, purpose: &str, selection: &Selection, r: &fr_render::Rendered, manifest: &mut Manifest) -> Result<(), CmdError> {
    let png = r.png_bytes()?;
    let path = dir.join(name);
    write_atomically(&path, &png)?;
    manifest.images.push(render_cmd::image_entry(&path, dir, purpose, selection, &png, r));
    Ok(())
}

/// Refuses an existing, non-empty directory without `--force`.
fn check_dir(dir: &Path, force: bool) -> Result<(), CmdError> {
    if dir.exists() && !dir.is_dir() {
        return Err(CmdError::output(format!("{}: exists and is not a directory", dir.display())));
    }
    if dir.is_dir() && !force {
        let non_empty = std::fs::read_dir(dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?.next().is_some();
        if non_empty {
            return Err(CmdError::output(format!("{}: already exists and is not empty; a pack is never written over another one (use --force to replace its files)", dir.display())));
        }
    }
    Ok(())
}

/// Creates the directory; with `--force` removes the previous pack files in it (only files with
/// the names of the pack, never the directory or anything else).
fn prepare_dir(dir: &Path, force: bool) -> Result<(), CmdError> {
    check_dir(dir, force)?;
    std::fs::create_dir_all(dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?;
    if force {
        for entry in std::fs::read_dir(dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if is_pack_file(&name) && entry.path().is_file() {
                std::fs::remove_file(entry.path()).map_err(|e| CmdError::output(format!("{}: {e}", entry.path().display())))?;
            }
        }
    }
    Ok(())
}

/// Everything wrong with the pack in `dir` (empty: consistent): every file the manifest names
/// exists with the stated size and hash, the JSON files parse, the statuses agree with the
/// files and with each other, and the issue ids in the pictures exist in `inspect.json`.
pub fn verify_pack(dir: &Path) -> Vec<String> {
    let mut problems = Vec::new();
    let mut bad = |m: String| problems.push(m);
    let manifest_path = dir.join(MANIFEST);
    let text = match std::fs::read_to_string(&manifest_path) {
        Ok(t) => t,
        Err(e) => return vec![format!("{}: {e}", manifest_path.display())],
    };
    let m: Manifest = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => return vec![format!("{MANIFEST}: not a manifest: {e}")],
    };
    if m.format != crate::manifest::FORMAT || m.format_version.split('.').next() != Some("1") {
        bad(format!("{MANIFEST}: format {} {} is not supported (1.x of fastroute-manifest)", m.format, m.format_version));
    }
    if m.status == "error" {
        bad(format!("{MANIFEST}: the pack records a failed run: {}", m.errors.iter().map(|e| e.message.as_str()).collect::<Vec<_>>().join("; ")));
        return problems;
    }
    for img in &m.images {
        let path = dir.join(&img.file);
        match std::fs::read(&path) {
            Err(e) => bad(format!("{}: {e}", img.file)),
            Ok(bytes) => {
                if bytes.len() as u64 != img.bytes {
                    bad(format!("{}: {} bytes, the manifest says {}", img.file, bytes.len(), img.bytes));
                }
                if load::sha256_hex(&bytes) != img.sha256 {
                    bad(format!("{}: the content does not match the sha256 in the manifest", img.file));
                }
                let dims = (bytes.len() > 24 && bytes[..8] == *b"\x89PNG\r\n\x1a\n").then(|| (u32::from_be_bytes(bytes[16..20].try_into().unwrap()), u32::from_be_bytes(bytes[20..24].try_into().unwrap())));
                if dims != Some((img.meta.width_px, img.meta.height_px)) {
                    bad(format!("{}: the PNG is {dims:?} pixels, the manifest says {} x {}", img.file, img.meta.width_px, img.meta.height_px));
                }
            }
        }
    }
    let mut ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut inspect_doc: Option<Value> = None;
    for (name, r) in &m.reports {
        match (&r.file, r.status.as_str()) {
            (Some(f), "present") => match std::fs::read_to_string(dir.join(f)) {
                Err(e) => bad(format!("reports.{name}: {f}: {e}")),
                Ok(t) => {
                    if r.bytes.is_some_and(|b| b != t.len() as u64) || r.sha256.as_deref().is_some_and(|h| h != load::sha256_hex(t.as_bytes())) {
                        bad(format!("reports.{name}: {f} does not match the size / sha256 in the manifest (changed after the pack was made)"));
                    }
                    match serde_json::from_str::<Value>(&t) {
                    Err(e) => bad(format!("reports.{name}: {f} is not valid JSON: {e}")),
                    Ok(v) => {
                        if name == "inspect" {
                            inspect_doc = Some(v);
                        }
                    }
                    }
                }
            },
            (None, "present") => bad(format!("reports.{name}: status \"present\" but no file")),
            (Some(f), s) => bad(format!("reports.{name}: status \"{s}\" but a file {f} is named")),
            (None, _) => {
                for stray in [(name == "drc_report").then_some(DRC_REPORT)].into_iter().flatten() {
                    if dir.join(stray).exists() {
                        bad(format!("reports.{name}: not present according to the manifest, but {stray} exists"));
                    }
                }
            }
        }
    }
    match &inspect_doc {
        None => bad("reports.inspect: missing".into()),
        Some(v) => {
            for key in ["unrouted", "violations"] {
                ids.extend(v[key].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(str::to_string));
            }
            ids.extend(v["checks"]["drc"]["issues"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_str()).map(str::to_string));
            let status = v["checks"]["drc"]["status"].as_str().unwrap_or("");
            if status != m.drc_status {
                bad(format!("drc_status \"{}\" in {MANIFEST} but \"{status}\" in {INSPECT}", m.drc_status));
            }
        }
    }
    let drc_present = m.reports.get("drc_report").is_some_and(|r| r.status == "present");
    if drc_present != matches!(m.drc_status.as_str(), "passed" | "failed") {
        bad(format!("drc_status \"{}\" does not fit the drc_report entry", m.drc_status));
    }
    for img in &m.images {
        for id in &img.meta.contains.issues {
            if !ids.contains(id) {
                bad(format!("{}: names the issue {id}, which {INSPECT} does not have", img.file));
            }
        }
    }
    problems
}
