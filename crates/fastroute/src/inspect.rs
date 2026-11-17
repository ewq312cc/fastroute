//! `fastroute inspect`: the board as structured data (`inspect.json`), for programs and AI
//! tools that should not have to read coordinates, nets and layers off a picture.
//!
//! Format: `docs/AI_DATA_FORMAT.md`, `docs/schema/inspect-1.0.schema.json`. Coordinates are
//! millimetres, x right, y down. The document never claims more than was done: the DRC part
//! says `not_run` unless a KiCad report was given, and a failed run is written as a document
//! with `"status": "error"` (not only to stderr).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fr_render::{BoardGeometry, Rect, RegionMm};
use serde::{Deserialize, Serialize};

use crate::cmd::{check_output, exit, opt, write_atomically, CmdError, Opt, Spec};
use crate::drc::{DrcIssue, DrcOutcome, DrcStatus};
use crate::extract::{self, Analysis, Unrouted, Violation};
use crate::load::{self, FileInfo, Loaded};
use crate::manifest::{CoordinateSystem, ErrorEntry};

pub const FORMAT: &str = "fastroute-inspect";
pub const FORMAT_VERSION: &str = "1.0";

fn r6(x: f64) -> f64 {
    if x.is_finite() {
        let r = (x * 1e6).round() / 1e6;
        if r == 0.0 {
            0.0
        } else {
            r
        }
    } else {
        0.0
    }
}

fn region(r: Rect) -> RegionMm {
    RegionMm { x: r6(r.x0), y: r6(r.y0), width: r6(r.width()), height: r6(r.height()) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerOut {
    pub index: usize,
    pub name: String,
    /// `signal` or `power`.
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BoardOut {
    pub name: String,
    pub outline_bbox_mm: Option<RegionMm>,
    /// Closed polygons; a hole in the board is its own polygon.
    pub outline: Vec<Vec<Point>>,
    pub layers: Vec<LayerOut>,
    pub component_count: usize,
    pub net_count: usize,
    pub pad_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComponentOut {
    #[serde(rename = "ref")]
    pub reference: String,
    pub side: String,
    pub x: f64,
    pub y: f64,
    pub rotation_deg: f64,
    pub bbox_mm: RegionMm,
    pub pad_count: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PadOut {
    pub component: String,
    pub pin: String,
    /// `null` for a pad without a net.
    pub net: Option<String>,
    pub layers: Vec<String>,
    pub x: f64,
    pub y: f64,
    /// `circle` or `polygon` (the shape on the first layer).
    pub shape: String,
    pub bbox_mm: RegionMm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NetOut {
    pub name: String,
    pub class: String,
    pub pad_count: usize,
    /// `complete`, `incomplete`, or `not_applicable` (fewer than two pads).
    pub status: String,
    pub unrouted_connections: usize,
    pub trace_count: usize,
    pub trace_length_mm: f64,
    pub via_count: usize,
    pub has_plane: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoutingOut {
    /// `complete` when no connection is unrouted, else `incomplete`. This is connectivity
    /// only; it says nothing about design rules (see `checks`).
    pub status: String,
    pub unrouted_connections: usize,
    pub unrouted_in_ignored_classes: usize,
    pub trace_count: usize,
    pub trace_length_mm: f64,
    pub via_count: usize,
    pub clearance_violations: usize,
    pub clearance_violations_unfixable: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LayerValue {
    pub layer: String,
    pub value_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClassOut {
    pub name: String,
    pub nets: Vec<String>,
    pub trace_width_mm: Vec<LayerValue>,
    pub clearance_mm: Vec<LayerValue>,
    pub ignored_by_autorouter: bool,
    pub min_length_mm: Option<f64>,
    pub max_length_mm: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RulesOut {
    /// Where the rules come from: `dsn`.
    pub source: String,
    pub file: String,
    pub net_classes: Vec<ClassOut>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckOut {
    pub tool: String,
    /// `completed`.
    pub status: String,
    pub count: usize,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DrcOut {
    /// `not_run`, `unavailable`, `error`, `timeout`, `failed` or `passed`. `passed` only
    /// from a readable KiCad report without violations and without unconnected items.
    pub status: String,
    pub tool: Option<String>,
    pub tool_version: Option<String>,
    pub detail: Option<String>,
    /// Path of the report this comes from, relative to inspect.json when it is next to it.
    pub report_file: Option<String>,
    pub violations: Option<u32>,
    pub unconnected: Option<u32>,
    pub errors: Option<u32>,
    pub warnings: Option<u32>,
    pub issues: Vec<DrcIssue>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChecksOut {
    pub connectivity: CheckOut,
    pub fastroute_clearance: CheckOut,
    pub drc: DrcOut,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inspect {
    pub format: String,
    pub format_version: String,
    pub fastroute_version: String,
    pub generated_by: String,
    /// `ok`, or `error` (then only `input`, `warnings` and `errors` are meaningful).
    pub status: String,
    pub input: Option<FileInfo>,
    pub session: Option<FileInfo>,
    pub coordinate_system: CoordinateSystem,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardOut>,
    /// Present (possibly empty) in every `ok` document; absent after an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub components: Option<Vec<ComponentOut>>,
    /// Present (possibly empty) in every `ok` document; absent after an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pads: Option<Vec<PadOut>>,
    /// Present (possibly empty) in every `ok` document; absent after an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nets: Option<Vec<NetOut>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub routing: Option<RoutingOut>,
    /// Present (possibly empty) in every `ok` document; absent after an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unrouted: Option<Vec<Unrouted>>,
    /// Present (possibly empty) in every `ok` document; absent after an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub violations: Option<Vec<Violation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<RulesOut>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<ChecksOut>,
    pub warnings: Vec<String>,
    pub errors: Vec<ErrorEntry>,
}

impl Inspect {
    pub fn to_json(&self) -> String {
        let mut s = serde_json::to_string_pretty(self).expect("inspect.json serialises");
        s.push('\n');
        s
    }

    /// The document of a failed run: what was asked for and why it did not work.
    pub fn failure(input: Option<FileInfo>, session: Option<FileInfo>, e: &CmdError) -> Inspect {
        Inspect {
            format: FORMAT.into(),
            format_version: FORMAT_VERSION.into(),
            fastroute_version: env!("CARGO_PKG_VERSION").into(),
            generated_by: "fastroute inspect".into(),
            status: "error".into(),
            input,
            session,
            coordinate_system: CoordinateSystem::standard(),
            board: None,
            components: None,
            pads: None,
            nets: None,
            routing: None,
            unrouted: None,
            violations: None,
            rules: None,
            checks: None,
            warnings: vec![],
            errors: vec![ErrorEntry { code: error_code(e.code).into(), message: e.message.clone() }],
        }
    }
}

/// The machine-readable name of an exit code.
pub fn error_code(code: u8) -> &'static str {
    match code {
        exit::USAGE => "usage",
        exit::INPUT => "input_invalid",
        exit::OUTPUT => "output",
        _ => "failed",
    }
}

/// Orders `U2` before `U10`: digit runs compare as numbers.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut x, mut y) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (x.peek().copied(), y.peek().copied()) {
            // equal as numbers and text of the same shape (`U02` / `U2`): fall back to the plain
            // string order, so that the order is total and never depends on the input order
            (None, None) => return a.cmp(b),
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(c), Some(d)) if c.is_ascii_digit() && d.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (n, m) = (take(&mut x), take(&mut y));
                let (n, m) = (n.trim_start_matches('0'), m.trim_start_matches('0'));
                match n.len().cmp(&m.len()).then_with(|| n.cmp(m)) {
                    std::cmp::Ordering::Equal => {}
                    o => return o,
                }
            }
            (Some(c), Some(d)) => {
                if c != d {
                    return c.cmp(&d);
                }
                x.next();
                y.next();
            }
        }
    }
}

fn pad_groups(g: &BoardGeometry) -> Vec<PadOut> {
    // one entry per (component, pin), with the layers it is on
    let mut order: Vec<(String, String)> = Vec::new();
    let mut groups: BTreeMap<(String, String), Vec<&fr_render::Pad>> = BTreeMap::new();
    for p in &g.pads {
        let key = (p.component.clone(), p.pin.clone());
        if !groups.contains_key(&key) {
            order.push(key.clone());
        }
        groups.entry(key).or_default().push(p);
    }
    order.sort_by(|a, b| natural_cmp(&a.0, &b.0).then_with(|| natural_cmp(&a.1, &b.1)));
    order
        .into_iter()
        .filter_map(|key| {
            let pads = &groups[&key];
            let boxes: Vec<Rect> = pads.iter().filter_map(|p| p.shape.bbox()).collect();
            let bbox = boxes.iter().skip(1).fold(*boxes.first()?, |a, b| a.union(b));
            let first = pads[0];
            // x, y: the pad's centre as the router uses it (the same point that unrouted
            // connections end at), see the docs for custom-shaped pads
            let c = first.center;
            Some(PadOut {
                component: key.0.clone(),
                pin: key.1.clone(),
                net: Some(g.net_name(first.net)).filter(|n| !n.is_empty()).map(str::to_string),
                layers: pads.iter().map(|p| g.layers[p.layer].name.clone()).collect(),
                x: r6(c.x),
                y: r6(c.y),
                shape: match first.shape {
                    fr_render::PadShape::Circle { .. } => "circle".into(),
                    fr_render::PadShape::Poly(_) => "polygon".into(),
                },
                bbox_mm: region(bbox),
            })
        })
        .collect()
}

fn round_unrouted(u: &Unrouted) -> Unrouted {
    let mut u = u.clone();
    u.length_mm = r6(u.length_mm);
    for e in [&mut u.from, &mut u.to] {
        e.x = r6(e.x);
        e.y = r6(e.y);
    }
    u
}

fn round_violation(v: &Violation) -> Violation {
    let mut v = v.clone();
    v.x = r6(v.x);
    v.y = r6(v.y);
    v.required_mm = r6(v.required_mm);
    v.actual_mm = r6(v.actual_mm);
    v
}

fn drc_out(o: &DrcOutcome, report_file: Option<String>) -> DrcOut {
    DrcOut {
        status: o.status.as_str().into(),
        tool: o.tool.clone(),
        tool_version: o.tool_version.clone(),
        detail: o.detail.clone(),
        report_file,
        violations: o.summary.as_ref().map(|s| s.violations),
        unconnected: o.summary.as_ref().map(|s| s.unconnected),
        errors: o.summary.as_ref().map(|s| s.errors),
        warnings: o.summary.as_ref().map(|s| s.warnings),
        issues: o.summary.as_ref().map(|s| s.issues.clone()).unwrap_or_default(),
    }
}

/// The document for a loaded board. `drc` is what is known about KiCad's checker (`not_run`
/// if nothing); `drc_report_file` is where its report is, for the reader.
pub fn build(loaded: &Loaded, a: &Analysis, drc: &DrcOutcome, drc_report_file: Option<String>) -> Inspect {
    let g = &a.geometry;
    let pads = pad_groups(g);

    let comp_pads = |name: &str| pads.iter().filter(|p| p.component == name).count();
    let mut paired: Vec<_> = g.components.iter().zip(&a.components).collect();
    paired.sort_by(|a, b| natural_cmp(&a.0.name, &b.0.name));
    let components: Vec<ComponentOut> = paired
        .into_iter()
        .map(|(c, info)| ComponentOut {
            reference: c.name.clone(),
            side: info.side.to_string(),
            x: r6(c.center.x),
            y: r6(c.center.y),
            rotation_deg: r6(info.rotation_deg),
            bbox_mm: region(c.bbox),
            pad_count: comp_pads(&c.name),
        })
        .collect();

    let lengths = extract::trace_length_per_net(g);
    let mut nets = Vec::new();
    for n in 1..g.nets.len() {
        if g.nets[n].is_empty() {
            continue;
        }
        let pad_count = pads.iter().filter(|p| p.net.as_deref() == Some(g.nets[n].as_str())).count();
        let unrouted = g.airlines.iter().filter(|l| l.net == n).count();
        nets.push(NetOut {
            name: g.nets[n].clone(),
            class: a.net_class.get(n).cloned().unwrap_or_default(),
            pad_count,
            status: if pad_count < 2 {
                "not_applicable"
            } else if unrouted == 0 {
                "complete"
            } else {
                "incomplete"
            }
            .into(),
            unrouted_connections: unrouted,
            trace_count: g.traces.iter().filter(|t| t.net == n).count(),
            trace_length_mm: r6(lengths.get(&n).copied().unwrap_or(0.0)),
            via_count: g.vias.iter().filter(|v| v.net == n).count(),
            has_plane: g.planes.iter().any(|p| p.net == n),
        });
    }

    let total_length: f64 = lengths.values().sum();
    let unrouted_ignored = a.unrouted.iter().filter(|u| u.ignored_class).count();
    let routing = RoutingOut {
        status: if a.unrouted.is_empty() { "complete" } else { "incomplete" }.into(),
        unrouted_connections: a.unrouted.len(),
        unrouted_in_ignored_classes: unrouted_ignored,
        trace_count: g.traces.len(),
        trace_length_mm: r6(total_length),
        via_count: g.vias.len(),
        clearance_violations: a.violations.len(),
        clearance_violations_unfixable: a.violations.iter().filter(|v| v.unfixable).count(),
    };

    let lv = |v: &extract::PerLayer| v.iter().map(|(l, x)| LayerValue { layer: l.clone(), value_mm: r6(*x) }).collect::<Vec<_>>();
    let rules = RulesOut {
        source: "dsn".into(),
        file: loaded.input.path.clone(),
        net_classes: a
            .classes
            .iter()
            .map(|c| ClassOut {
                name: c.name.clone(),
                nets: c.nets.clone(),
                trace_width_mm: lv(&c.trace_width_mm),
                clearance_mm: lv(&c.clearance_mm),
                ignored_by_autorouter: c.ignored_by_autorouter,
                min_length_mm: c.min_length_mm.map(r6),
                max_length_mm: c.max_length_mm.map(r6),
            })
            .collect(),
        note: "Rules of the Specctra design: net classes with trace widths and trace-to-trace clearances. Other clearances (pad, via, keepout, per item pair) and KiCad custom rules (.kicad_dru) are applied by the router but not listed here unless the DSN carries them as net class rules.".into(),
    };

    Inspect {
        format: FORMAT.into(),
        format_version: FORMAT_VERSION.into(),
        fastroute_version: env!("CARGO_PKG_VERSION").into(),
        generated_by: "fastroute inspect".into(),
        status: "ok".into(),
        input: Some(loaded.input.clone()),
        session: loaded.session.clone(),
        coordinate_system: CoordinateSystem::standard(),
        board: Some(BoardOut {
            name: g.name.clone(),
            outline_bbox_mm: g.outline_bbox().map(region),
            outline: g.outline.iter().map(|poly| poly.iter().map(|p| Point { x: r6(p.x), y: r6(p.y) }).collect()).collect(),
            layers: g.layers.iter().enumerate().map(|(i, l)| LayerOut { index: i, name: l.name.clone(), kind: if l.signal { "signal" } else { "power" }.into() }).collect(),
            component_count: g.components.len(),
            net_count: nets.len(),
            pad_count: pads.len(),
        }),
        components: Some(components),
        pads: Some(pads),
        nets: Some(nets),
        routing: Some(routing),
        unrouted: Some(a.unrouted.iter().map(round_unrouted).collect()),
        violations: Some(a.violations.iter().map(round_violation).collect()),
        rules: Some(rules),
        checks: Some(ChecksOut {
            connectivity: CheckOut {
                tool: "fastroute".into(),
                status: "completed".into(),
                count: a.unrouted.len(),
                detail: "unrouted connections: groups of a net that are not joined by copper (airlines)".into(),
            },
            fastroute_clearance: CheckOut {
                tool: "fastroute".into(),
                status: "completed".into(),
                count: a.violations.len(),
                detail: "clearance between copper items of the loaded wiring, as the router measures it; this is not KiCad's DRC".into(),
            },
            drc: drc_out(drc, drc_report_file),
        }),
        warnings: loaded.warnings.clone(),
        errors: vec![],
    }
}

const OPTS: &[Opt] = &[
    opt("input", Some('i'), Some("FILE"), "Specctra design (.dsn) to describe (also the first positional argument)"),
    opt("session", Some('s'), Some("FILE"), "session (.ses) whose wiring is part of the board, e.g. the result of a route run"),
    opt("output", Some('o'), Some("FILE"), "inspect.json to write ('-' = standard output); an existing file needs --force"),
    opt("drc-report", None, Some("FILE"), "a KiCad DRC report (kicad-cli pcb drc --format json --units mm) to include; without it\nthe DRC status is \"not_run\", never \"passed\""),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary line"),
];

pub const SPEC: Spec = Spec {
    name: "inspect",
    about: "describe the board as structured JSON (inspect.json)",
    usage: "inspect --input design.dsn --output inspect.json [options]",
    opts: OPTS,
    passthrough: &[],
    footer: "inspect.json lists the board outline, layers, components, pads, nets, the routing state\n\
             (unrouted connections with coordinates, clearance violations), the design rules and the\n\
             status of every check. Coordinates are mm, x right, y down. If the run fails, the file\n\
             (when an output was given) holds {\"status\": \"error\", \"errors\": [...]} and the exit code\n\
             is not 0. Format: docs/AI_DATA_FORMAT.md.",
};

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
        (None, []) => return Err(CmdError::usage("no input design (--input design.dsn)")),
        (None, _) => return Err(CmdError::usage("more than one input design given")),
    };
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output file (--output inspect.json, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    let session = p.path("session");
    let drc_report = p.path("drc-report");
    let force = p.flag("force");
    let mut inputs: Vec<&Path> = vec![&input];
    inputs.extend(session.as_deref());
    inputs.extend(drc_report.as_deref());
    if !to_stdout {
        check_output(&output, &inputs, force)?;
    }

    let outcome = build_document(&input, session.as_deref(), drc_report.as_deref());
    let (doc, result) = match outcome {
        Ok(doc) => (doc, Ok(())),
        Err(failed) => {
            let (doc, e) = *failed;
            (doc, Err(e))
        }
    };
    let json = doc.to_json();
    if to_stdout {
        print!("{json}");
    } else {
        write_atomically(&output, json.as_bytes())?;
    }
    if result.is_ok() && !p.flag("quiet") && !to_stdout {
        let r = doc.routing.as_ref().expect("an ok document has a routing summary");
        println!(
            "wrote {}: {} components, {} nets, {} unrouted connections, {} clearance violations, DRC {}",
            output.display(),
            doc.components.as_ref().map_or(0, Vec::len),
            doc.nets.as_ref().map_or(0, Vec::len),
            r.unrouted_connections,
            r.clearance_violations,
            doc.checks.as_ref().map(|c| c.drc.status.as_str()).unwrap_or("not_run")
        );
        for w in &doc.warnings {
            eprintln!("warning: {w}");
        }
    }
    result
}

/// Loads and describes; on failure returns the error document together with the error.
fn build_document(input: &Path, session: Option<&Path>, drc_report: Option<&Path>) -> Result<Inspect, Box<(Inspect, CmdError)>> {
    let file_info = |p: &Path| std::fs::read(p).ok().map(|d| FileInfo::of(p, &d));
    let fail = |e: CmdError| Box::new((Inspect::failure(file_info(input), session.and_then(file_info), &e), e));

    let drc = match drc_report {
        Some(p) => match std::fs::read_to_string(p) {
            Ok(text) => DrcOutcome::from_report_text(text, &p.display().to_string()),
            Err(e) => return Err(fail(CmdError::input(format!("{}: {e}", p.display())))),
        },
        None => DrcOutcome::not_run("inspect does not run KiCad's DRC; use `fastroute ai-pack --kicad-pcb board.kicad_pcb` or pass a report with --drc-report"),
    };
    if drc.status == DrcStatus::Error {
        // a report that was asked for but cannot be read is the caller's mistake, not a "not run"
        return Err(fail(CmdError::input(drc.detail.clone().unwrap_or_else(|| "unreadable DRC report".into()))));
    }
    let loaded = load::load(input, session).map_err(fail)?;
    let analysis = extract::analyse(&loaded.board, &loaded.name, &loaded.synthetic);
    Ok(build(&loaded, &analysis, &drc, drc_report.map(|p| p.display().to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut v = vec!["U10", "U2", "R1", "U1", "C100", "C20", "1", "10", "2", "A1", "A", "U02"];
        v.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(v, ["1", "2", "10", "A", "A1", "C20", "C100", "R1", "U1", "U02", "U2", "U10"]);
    }

    #[test]
    fn rounding_hides_float_noise_and_negative_zero() {
        assert_eq!(r6(10.8 + 1e-12), 10.8);
        assert_eq!(r6(-0.0000001), 0.0);
        assert!(r6(-0.0000001).is_sign_positive());
        assert_eq!(r6(f64::NAN), 0.0);
    }

    #[test]
    fn exit_codes_have_machine_readable_names() {
        assert_eq!(error_code(exit::USAGE), "usage");
        assert_eq!(error_code(exit::INPUT), "input_invalid");
        assert_eq!(error_code(exit::OUTPUT), "output");
        assert_eq!(error_code(exit::FAILED), "failed");
    }
}
