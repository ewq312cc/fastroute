//! Reading problem locations out of reports: fastroute's own `--report` JSON, `inspect.json`
//! and KiCad's `kicad-cli pcb drc --format json` output. The locations become
//! [`fr_render::Issue`]s, so they can select a region and be drawn as numbered markers.
//!
//! Coordinates are taken as they are: all three formats use millimetres with y pointing down,
//! and KiCad's board coordinates are the coordinates of the exported DSN (that is how the KiCad
//! plugin and Specctra export work), so no transform is applied.

use std::path::Path;

use fr_render::{Issue, IssueKind, Pt, Rect};
use serde_json::Value;

use crate::cmd::CmdError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    /// `fastroute --report=FILE`
    FastrouteReport,
    /// `fastroute inspect` output
    Inspect,
    /// `kicad-cli pcb drc --format json`
    KicadDrc,
}

#[derive(Debug)]
pub struct ParsedReport {
    pub kind: ReportKind,
    pub issues: Vec<Issue>,
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

fn xy_array(v: &Value) -> Option<Pt> {
    let a = v.as_array()?;
    Some(Pt::new(num(a.first()?)?, num(a.get(1)?)?))
}

fn xy_object(v: &Value) -> Option<Pt> {
    Some(Pt::new(num(v.get("x")?)?, num(v.get("y")?)?))
}

fn connection(id: String, from: Pt, to: Pt) -> Issue {
    Issue { extent: Rect::from_points([&from, &to]).expect("finite"), ..Issue::at_point(id, IssueKind::Unrouted, Pt::new((from.x + to.x) / 2.0, (from.y + to.y) / 2.0)) }
}

fn violation(id: String, at: Pt) -> Issue {
    Issue::at_point(id, IssueKind::Violation, at)
}

/// Reads a report file; the format is recognised from its content.
pub fn read_report(path: &Path) -> Result<ParsedReport, CmdError> {
    let text = std::fs::read_to_string(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    parse_report(&text).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))
}

pub fn parse_report(text: &str) -> Result<ParsedReport, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let obj = v.as_object().ok_or("not a JSON object")?;
    let mut issues = Vec::new();
    let kind;
    if obj.get("format").and_then(Value::as_str) == Some("fastroute-inspect") {
        kind = ReportKind::Inspect;
        if obj.get("status").and_then(Value::as_str) == Some("error") {
            return Err("this inspect.json records a failed run (status \"error\"), it has no locations".into());
        }
        for (i, u) in v["unrouted"].as_array().into_iter().flatten().enumerate() {
            let id = u["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("OPEN{}", i + 1));
            let (Some(a), Some(b)) = (xy_object(&u["from"]), xy_object(&u["to"])) else { return Err(format!("unrouted[{i}] has no from/to coordinates")) };
            issues.push(connection(id, a, b));
        }
        for (i, x) in v["violations"].as_array().into_iter().flatten().enumerate() {
            let id = x["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("CLR{}", i + 1));
            let Some(at) = xy_object(x) else { return Err(format!("violations[{i}] has no coordinates")) };
            issues.push(violation(id, at));
        }
        for (i, x) in v["drc"]["issues"].as_array().into_iter().flatten().enumerate() {
            let id = x["id"].as_str().map(str::to_string).unwrap_or_else(|| format!("DRC{}", i + 1));
            let Some(at) = xy_object(x) else { return Err(format!("drc.issues[{i}] has no coordinates")) };
            issues.push(Issue::at_point(id, IssueKind::Drc, at));
        }
    } else if obj.contains_key("clearance_violations") || (obj.contains_key("unrouted") && obj.contains_key("fastroute")) {
        kind = ReportKind::FastrouteReport;
        for (i, u) in v["unrouted"].as_array().into_iter().flatten().enumerate() {
            let (Some(a), Some(b)) = (xy_array(&u["from_xy"]), xy_array(&u["to_xy"])) else { return Err(format!("unrouted[{i}] has no from_xy/to_xy")) };
            issues.push(connection(format!("OPEN{}", i + 1), a, b));
        }
        for (i, x) in v["clearance_violations"].as_array().into_iter().flatten().enumerate() {
            let Some(at) = xy_array(&x["xy"]) else { return Err(format!("clearance_violations[{i}] has no xy")) };
            issues.push(violation(format!("CLR{}", i + 1), at));
        }
    } else if obj.contains_key("violations") || obj.contains_key("unconnected_items") {
        kind = ReportKind::KicadDrc;
        issues = kicad_drc_issues(&v)?;
    } else {
        return Err("unrecognised report: expected fastroute --report JSON, an inspect.json or KiCad DRC JSON".into());
    }
    Ok(ParsedReport { kind, issues })
}

/// `DRC1`, `DRC2`, ... for the violations and then the unconnected items of a KiCad DRC report.
pub fn kicad_drc_issues(v: &Value) -> Result<Vec<Issue>, String> {
    let mut issues = Vec::new();
    for key in ["violations", "unconnected_items"] {
        for (i, x) in v[key].as_array().into_iter().flatten().enumerate() {
            let pts: Vec<Pt> = x["items"].as_array().into_iter().flatten().filter_map(|it| xy_object(&it["pos"])).collect();
            let Some(extent) = Rect::from_points(&pts) else {
                return Err(format!("{key}[{i}] has no item with a position"));
            };
            let c = extent.center();
            issues.push(Issue { extent, ..Issue::at_point(format!("DRC{}", issues.len() + 1), IssueKind::Drc, c) });
        }
    }
    Ok(issues)
}

/// Selects issues for `--select`: `all`, `unrouted`, `violations`, `drc`, or ids (`OPEN1,CLR3`),
/// comma separated.
pub fn select_issues<'a>(issues: &'a [Issue], spec: &str) -> Result<Vec<&'a Issue>, String> {
    let mut out: Vec<&Issue> = Vec::new();
    for w in spec.split(',').map(str::trim).filter(|w| !w.is_empty()) {
        match w.to_ascii_lowercase().as_str() {
            "all" => out.extend(issues.iter()),
            "unrouted" => out.extend(issues.iter().filter(|i| i.kind == IssueKind::Unrouted)),
            "violations" | "clearance" => out.extend(issues.iter().filter(|i| i.kind == IssueKind::Violation)),
            "drc" => out.extend(issues.iter().filter(|i| i.kind == IssueKind::Drc)),
            _ => match issues.iter().find(|i| i.id.eq_ignore_ascii_case(w)) {
                Some(i) => out.push(i),
                None => {
                    let known: Vec<&str> = issues.iter().take(6).map(|i| i.id.as_str()).collect();
                    return Err(format!("no issue '{w}' (known: {}{})", known.join(", "), if issues.len() > 6 { ", ..." } else { "" }));
                }
            },
        }
    }
    if out.is_empty() {
        return Err(format!("nothing to select for '{spec}' (the report has no such issues)"));
    }
    out.dedup_by(|a, b| a.id == b.id);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fastroute_report() {
        let r = parse_report(
            r#"{"fastroute":"0.1.14","unrouted":[{"net":"A","from_xy":[1,2],"to_xy":[5,4]}],"clearance_violations":[{"layer":"F.Cu","xy":[7.5,8.5]}]}"#,
        )
        .unwrap();
        assert_eq!(r.kind, ReportKind::FastrouteReport);
        assert_eq!(r.issues.len(), 2);
        assert_eq!(r.issues[0].id, "OPEN1");
        assert_eq!((r.issues[0].extent.x0, r.issues[0].extent.y1), (1.0, 4.0));
        assert_eq!((r.issues[0].at.x, r.issues[0].at.y), (3.0, 3.0));
        assert_eq!((r.issues[1].id.as_str(), r.issues[1].at.x), ("CLR1", 7.5));
    }

    #[test]
    fn inspect_report_keeps_its_ids() {
        let r = parse_report(
            r#"{"format":"fastroute-inspect","unrouted":[{"id":"OPEN7","from":{"x":0,"y":0},"to":{"x":2,"y":2}}],
                "violations":[{"id":"CLR3","x":1,"y":1}],"drc":{"issues":[{"id":"DRC9","x":4,"y":4}]}}"#,
        )
        .unwrap();
        assert_eq!(r.kind, ReportKind::Inspect);
        assert_eq!(r.issues.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["OPEN7", "CLR3", "DRC9"]);
        let failed = parse_report(r#"{"format":"fastroute-inspect","status":"error"}"#).unwrap_err();
        assert!(failed.contains("failed run"), "{failed}");
    }

    #[test]
    fn kicad_drc_report() {
        let r = parse_report(
            r#"{"$schema":"https://schemas.kicad.org/drc.v1.json","kicad_version":"9.0.0","coordinate_units":"mm",
                "violations":[{"type":"clearance","severity":"error","description":"x",
                   "items":[{"description":"Track","pos":{"x":10.0,"y":20.0}},{"description":"Pad","pos":{"x":12.0,"y":20.5}}]}],
                "unconnected_items":[{"type":"unconnected_items","items":[{"pos":{"x":1,"y":2}},{"pos":{"x":3,"y":4}}]}],
                "schematic_parity":[]}"#,
        )
        .unwrap();
        assert_eq!(r.kind, ReportKind::KicadDrc);
        assert_eq!(r.issues.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["DRC1", "DRC2"]);
        assert!(r.issues.iter().all(|i| i.kind == IssueKind::Drc));
        assert_eq!((r.issues[0].extent.x0, r.issues[0].extent.x1), (10.0, 12.0));
        assert_eq!((r.issues[0].at.x, r.issues[0].at.y), (11.0, 20.25));
    }

    #[test]
    fn bad_reports_are_errors() {
        for (text, needle) in [
            ("not json", "not valid JSON"),
            ("[1]", "not a JSON object"),
            ("{}", "unrecognised report"),
            (r#"{"violations":[{"items":[]}]}"#, "no item with a position"),
            (r#"{"fastroute":"1","unrouted":[{"from_xy":[1]}]}"#, "no from_xy"),
        ] {
            let e = parse_report(text).err().unwrap_or_default();
            assert!(e.contains(needle), "{text}: {e}");
        }
    }

    #[test]
    fn selecting() {
        let issues = parse_report(r#"{"fastroute":"1","unrouted":[{"from_xy":[0,0],"to_xy":[1,1]},{"from_xy":[5,5],"to_xy":[6,6]}],"clearance_violations":[{"xy":[9,9]}]}"#).unwrap().issues;
        assert_eq!(select_issues(&issues, "all").unwrap().len(), 3);
        assert_eq!(select_issues(&issues, "unrouted").unwrap().len(), 2);
        assert_eq!(select_issues(&issues, "violations").unwrap()[0].id, "CLR1");
        assert_eq!(select_issues(&issues, "open2, clr1").unwrap().iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["OPEN2", "CLR1"]);
        assert!(select_issues(&issues, "drc").unwrap_err().contains("nothing to select"));
        assert!(select_issues(&issues, "OPEN9").unwrap_err().contains("no issue 'OPEN9'"));
    }
}
