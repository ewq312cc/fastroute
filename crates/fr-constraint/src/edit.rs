//! A fixed base design with regions that an AI may edit.
//!
//! The pack says what may change (`base_design`): everything else is read-only. An AI does not redraw
//! the board; it hands over a draft (`fastroute-hcp-edit-draft`): a list of changes, each inside a
//! region, bound to the exact knowledge pack and base design it was made for. This module computes the
//! whitelist from the compiled IR (no AI, no network, no vector index: the same pack gives the same
//! whitelist), writes the generation request, checks a draft against the whitelist and the IR's rules,
//! and says what would change, with the evidence behind each decision. It never writes to the base
//! project: the base is read, the draft is read, and the only output is a report.

use crate::diag::Diagnostic;
use crate::geom::{self, Pt};
use crate::hcp::{canonical_json, protected_pins_of, sha256_hex};
use crate::ir::{Ir, IrEvidence};
use crate::jsonschema;
use crate::report::{t, Lang};
use crate::snapshot::{Component, PinUse, Snapshot, Track, Via};
use crate::validate::{self, Verdict};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const DRAFT_FORMAT: &str = "fastroute-hcp-edit-draft";
pub const REQUEST_FORMAT: &str = "fastroute-hcp-edit-request";
pub const CHECK_FORMAT: &str = "fastroute-hcp-edit-check";
pub const DRAFT_VERSION: &str = "1.0";

pub const DRAFT_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/edit-draft-v1.schema.json");
pub const REQUEST_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/edit-request-v1.schema.json");

pub fn draft_schema() -> Value {
    serde_json::from_str(DRAFT_SCHEMA_TEXT).expect("the embedded draft schema is valid JSON")
}

pub fn request_schema() -> Value {
    serde_json::from_str(REQUEST_SCHEMA_TEXT).expect("the embedded request schema is valid JSON")
}

// ---- the whitelist --------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AuthRegion {
    pub id: String,
    pub title: Option<String>,
    pub polygon: Vec<Pt>,
    pub layers: Vec<String>,
    pub allowed_blocks: Vec<String>,
    /// device id -> the pins an edit may use (the declared pool without the protected pins)
    pub pin_pool: BTreeMap<String, Vec<String>>,
    /// Only what is listed is allowed: no list, no operation.
    pub allowed_ops: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Protected {
    pub rule_id: String,
    pub kind: String,
}

#[derive(Debug, Clone)]
pub struct FixedRegion {
    pub id: String,
    pub kind: String,
    pub polygon: Vec<Pt>,
    pub layers: Vec<String>,
    pub evidence: Vec<IrEvidence>,
}

#[derive(Debug, Clone)]
pub struct Authorization {
    pub base_id: String,
    pub title: Option<String>,
    pub binding: Value,
    pub binding_sha256: String,
    pub regions: Vec<AuthRegion>,
    pub immutable_components: BTreeSet<String>,
    pub immutable_pins: BTreeSet<String>,
    pub fixed_regions: Vec<FixedRegion>,
    pub board_outline_fixed: bool,
    pub mount_holes_fixed: bool,
    /// device id -> pin -> the rule (or the pin table) that protects it
    pub protected: BTreeMap<String, BTreeMap<String, Protected>>,
}

fn strs(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// Computes what may be changed from the IR. `None` when the pack has no `base_design`: then nothing is
/// editable.
pub fn authorization(ir: &Ir) -> Option<Authorization> {
    let base = ir.base_design.as_ref()?;
    let binding = base.get("binding").cloned().unwrap_or(Value::Null);
    let binding_sha256 = sha256_hex(canonical_json(&binding).as_bytes());

    let mut protected: BTreeMap<String, BTreeMap<String, Protected>> = BTreeMap::new();
    for r in ir.rules.iter().filter(|r| r.status != "unknown" && r.status != "conflict") {
        if let Some(dev) = r.subject.get("device").and_then(Value::as_str) {
            for pin in protected_pins_of(&r.kind, &r.subject, &r.params) {
                protected.entry(dev.to_string()).or_default().entry(pin).or_insert(Protected { rule_id: r.rule_id.clone(), kind: r.kind.clone() });
            }
        }
    }
    for d in &ir.devices {
        for p in d.pins.iter().filter(|p| matches!(p.class.as_str(), "power" | "ground" | "strapping" | "reset" | "boot" | "rf" | "nc" | "reserved")) {
            protected.entry(d.id.clone()).or_default().entry(p.number.clone()).or_insert(Protected { rule_id: format!("pin table of {}", d.id), kind: format!("pin class {}", p.class) });
        }
    }

    let mut regions = Vec::new();
    for e in base.get("editable_regions").and_then(Value::as_array).into_iter().flatten() {
        let Some(id) = e.get("region").and_then(Value::as_str) else { continue };
        let Some(g) = ir.region(id).filter(|g| g.frame == "board") else { continue };
        let Some(polygon) = g.polygon.clone() else { continue };
        let mut pool: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for p in e.get("pin_pool").and_then(Value::as_array).into_iter().flatten() {
            let Some(dev) = p.get("device").and_then(Value::as_str) else { continue };
            let pins: Vec<String> = strs(p.get("pins")).into_iter().filter(|pin| !protected.get(dev).is_some_and(|m| m.contains_key(pin))).collect();
            pool.entry(dev.to_string()).or_default().extend(pins);
        }
        for v in pool.values_mut() {
            v.sort();
            v.dedup();
        }
        regions.push(AuthRegion {
            id: id.to_string(),
            title: None,
            polygon,
            layers: g.layers.clone(),
            allowed_blocks: strs(e.get("allowed_blocks")),
            pin_pool: pool,
            allowed_ops: strs(e.get("allowed_ops")),
        });
    }

    let imm = base.get("immutable").cloned().unwrap_or(Value::Null);
    let mut immutable_components: BTreeSet<String> = strs(imm.get("components")).into_iter().collect();
    immutable_components.extend(strs(imm.get("connectors")));
    let immutable_regions = strs(imm.get("regions"));
    let editable: BTreeSet<&str> = regions.iter().map(|r| r.id.as_str()).collect();
    let mut fixed_regions = Vec::new();
    for g in &ir.regions {
        let keepout = g.kind.contains("keepout");
        if (keepout || immutable_regions.contains(&g.id)) && g.kind != "board_outline" && !editable.contains(g.id.as_str()) && g.frame == "board" {
            if let Some(polygon) = g.polygon.clone() {
                fixed_regions.push(FixedRegion { id: g.id.clone(), kind: g.kind.clone(), polygon, layers: g.layers.clone(), evidence: g.evidence.clone() });
            }
        }
    }
    Some(Authorization {
        base_id: base.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
        title: base.get("title").and_then(Value::as_str).map(str::to_string),
        binding,
        binding_sha256,
        regions,
        immutable_components,
        immutable_pins: strs(imm.get("pins")).into_iter().collect(),
        fixed_regions,
        board_outline_fixed: imm.get("board_outline").and_then(Value::as_bool).unwrap_or(false),
        mount_holes_fixed: imm.get("mount_holes").and_then(Value::as_bool).unwrap_or(false),
        protected,
    })
}

/// The request for a semantic generation run: everything an AI is allowed to know about its authority,
/// computed from the pack. Deterministic.
pub fn edit_request(ir: &Ir, requirement: &str) -> Result<Value, String> {
    if !ir.verify_integrity() {
        return Err("the Constraint IR does not match its integrity hash: compile the pack again".into());
    }
    let auth = authorization(ir).ok_or_else(|| "the pack has no base_design: nothing in this design may be edited".to_string())?;
    let regions: Vec<Value> = auth
        .regions
        .iter()
        .map(|r| {
            let blocks: Vec<Value> = r
                .allowed_blocks
                .iter()
                .filter_map(|b| ir.block(b))
                .map(|b| json!({"id": b.id, "title": b.title, "kind": b.kind, "components": b.components, "connects": b.connects, "evidence": b.evidence}))
                .collect();
            json!({
                "id": r.id,
                "polygon": r.polygon,
                "layers": r.layers,
                "allowed_ops": r.allowed_ops,
                "blocks": blocks,
                "pin_pool": r.pin_pool.iter().map(|(d, p)| json!({"device": d, "pins": p})).collect::<Vec<_>>(),
            })
        })
        .collect();
    let protected: Vec<Value> = auth
        .protected
        .iter()
        .flat_map(|(d, m)| m.iter().map(move |(p, why)| json!({"device": d, "pin": p, "rule": why.rule_id, "kind": why.kind})))
        .collect();
    let rules: Vec<Value> = ir.rules.iter().filter(|r| r.origin == "hcp").map(|r| json!({"id": r.rule_id, "kind": r.kind, "modifiability": r.modifiability, "status": r.status, "title": r.title})).collect();
    Ok(json!({
        "format": REQUEST_FORMAT,
        "request_version": "1.0",
        "pack": {"id": ir.pack.id, "version": ir.pack.version, "sha256": ir.pack.sha256, "review": ir.pack.review},
        "base": {"id": auth.base_id, "title": auth.title, "binding": auth.binding, "binding_sha256": auth.binding_sha256},
        "requirement": requirement,
        "authorization": {
            "regions": regions,
            "immutable": {
                "components": auth.immutable_components,
                "pins": auth.immutable_pins,
                "board_outline": auth.board_outline_fixed,
                "mount_holes": auth.mount_holes_fixed,
            },
            "fixed_regions": auth.fixed_regions.iter().map(|g| json!({"id": g.id, "kind": g.kind, "polygon": g.polygon, "layers": g.layers})).collect::<Vec<_>>(),
            "protected_pins": protected,
        },
        "rules": rules,
        "contract": {
            "draft_format": DRAFT_FORMAT,
            "draft_schema": "schemas/edit-draft-v1.schema.json",
            "instructions": [
                "Answer with one edit draft (JSON). Do not write the KiCad project and do not redraw the board.",
                "Copy pack.sha256 and base.binding_sha256 into the draft: a draft for another version is sent back for review.",
                "Every change names the region it is in. Use only the regions, blocks, pins and operations listed here; anything else is refused. Search results (keyword or vector) do not add to this list.",
                "Fixed components, pins, the outline, mounting holes, connectors, keep-outs and protected pins are not touched.",
                "If the requirement cannot be met inside the whitelist, or a document is missing, answer with `stop` (reason, what is missing) and no changes. Do not widen the authority.",
            ],
        },
    }))
}

// ---- the check ------------------------------------------------------------------------------------

/// A digest of a file of the base design, as found in the working copy.
#[derive(Debug, Clone)]
pub struct BaseFile {
    pub path: String,
    /// `None`: the file is not there.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditFinding {
    pub code: String,
    /// `error` or `warning`
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<IrEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeResult {
    pub id: String,
    pub op: String,
    pub region: String,
    /// `accepted` or `rejected`
    pub verdict: &'static str,
    pub codes: Vec<String>,
    /// What authorises it: the whitelist entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authorized_by: Option<String>,
    pub before: Value,
    pub after: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<IrEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EditCheck {
    pub format: &'static str,
    pub format_version: &'static str,
    pub fastroute_version: &'static str,
    /// `accepted`, `rejected`, `needs_review` or `stopped`
    pub status: &'static str,
    /// Whether nothing was left unchecked (the base was verified, the validator ran).
    pub complete: bool,
    pub pack: Value,
    pub base: Value,
    pub summary: Value,
    pub review_reasons: Vec<String>,
    pub findings: Vec<EditFinding>,
    pub changes: Vec<ChangeResult>,
    pub diff: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<Value>,
}

const REVIEW_CODES: &[&str] = &["pack_mismatch", "base_mismatch", "base_changed"];

struct Added {
    region: String,
}

struct Checker<'a> {
    ir: &'a Ir,
    auth: Option<&'a Authorization>,
    base: Option<&'a Snapshot>,
    draft: &'a Value,
    ref_device: BTreeMap<String, String>,
    protected_nets: BTreeMap<String, String>,
    added: BTreeMap<String, Added>,
    removed: BTreeSet<String>,
    /// pins connected by the draft: (ref, pin) -> net
    connected: BTreeMap<(String, String), String>,
    findings: Vec<EditFinding>,
}

impl<'a> Checker<'a> {
    fn push(&mut self, code: &str, severity: &str, change: Option<&str>, object: Option<String>, message: impl Into<String>, evidence: Vec<IrEvidence>) {
        self.findings.push(EditFinding { code: code.into(), severity: severity.into(), change: change.map(str::to_string), object, message: message.into(), evidence });
    }

    fn err(&mut self, ch: &str, code: &str, object: Option<String>, message: impl Into<String>) {
        self.push(code, "error", Some(ch), object, message, vec![]);
    }

    fn err_ev(&mut self, ch: &str, code: &str, object: Option<String>, message: impl Into<String>, evidence: Vec<IrEvidence>) {
        self.push(code, "error", Some(ch), object, message, evidence);
    }

    fn region_evidence(&self, id: &str) -> Vec<IrEvidence> {
        self.ir.region(id).map(|g| g.evidence.clone()).unwrap_or_default()
    }

    fn rule_evidence(&self, id: &str) -> Vec<IrEvidence> {
        self.ir.rule(id).map(|r| r.evidence.clone()).unwrap_or_default()
    }

    /// Checks that a point may carry copper or a part of the region: inside the region, outside every
    /// fixed region and inside the board outline.
    fn place(&mut self, ch: &str, region: &AuthRegion, p: Pt, margin: f64, layer: Option<&str>, what: &str) -> bool {
        let mut ok = true;
        if !geom::point_in_polygon(p, &region.polygon) {
            self.err(ch, "outside_region", Some(what.to_string()), format!("{what} at ({}, {}) mm is outside {}", p[0], p[1], region.id));
            return false;
        }
        let edge = (0..region.polygon.len()).map(|i| geom::point_segment_distance(p, region.polygon[i], region.polygon[(i + 1) % region.polygon.len()])).fold(f64::INFINITY, f64::min);
        if margin > 0.0 && edge + 1e-9 < margin {
            self.err(ch, "crosses_region", Some(what.to_string()), format!("{what} at ({}, {}) mm reaches {:.3} mm beyond the border of {}", p[0], p[1], margin - edge, region.id));
            ok = false;
        }
        let fixed: Vec<FixedRegion> = self.auth.map(|a| a.fixed_regions.clone()).unwrap_or_default();
        for g in fixed.iter().filter(|g| layer.is_none_or(|l| g.layers.iter().any(|x| x == "all" || x == l))) {
            if geom::point_polygon_distance(p, &g.polygon) <= margin + 1e-9 {
                self.err_ev(ch, "enters_keepout", Some(what.to_string()), format!("{what} at ({}, {}) mm is in {} ({}), which is fixed", p[0], p[1], g.id, g.kind), g.evidence.clone());
                ok = false;
            }
        }
        if let Some(b) = self.base.filter(|b| b.outline.len() >= 3) {
            if !geom::point_in_polygon(p, &b.outline) {
                self.err(ch, "outside_board", Some(what.to_string()), format!("{what} at ({}, {}) mm is outside the board outline", p[0], p[1]));
                ok = false;
            }
        }
        ok
    }

    /// A base component that an edit may touch: not listed as immutable, not a device (its pins belong
    /// to the pin pool), inside the region.
    fn touchable(&mut self, ch: &str, region: &AuthRegion, reference: &str, uuid: Option<&str>) -> Option<Component> {
        let auth = self.auth?;
        if auth.immutable_components.contains(reference) {
            self.err(ch, "immutable_object", Some(reference.into()), format!("{reference} is a fixed component of the base design (immutable)"));
            return None;
        }
        let Some(base) = self.base else {
            self.err(ch, "base_required", Some(reference.into()), format!("{reference} is part of the base design: the design (--design or --kicad-pcb) is needed to check a change to it"));
            return None;
        };
        let Some(c) = base.component(reference).cloned() else {
            self.err(ch, "ref_unknown", Some(reference.into()), format!("{reference} is neither in the base design nor added by this draft"));
            return None;
        };
        if let (Some(want), Some(have)) = (uuid, c.uuid.as_deref()) {
            if want != have {
                self.err(ch, "stale_reference", Some(reference.into()), format!("{reference} has the identity {have} in the base design, the draft means {want}: the references changed since the draft was made"));
                return None;
            }
        }
        if self.ref_device.contains_key(reference) {
            self.err(ch, "base_object_modified", Some(reference.into()), format!("{reference} is a device of the pack: its pins are changed only through the pin pool, and it is not moved, removed or disconnected"));
            return None;
        }
        if c.locked {
            self.err(ch, "immutable_object", Some(reference.into()), format!("{reference} is locked in the base design"));
            return None;
        }
        if !geom::point_in_polygon([c.x, c.y], &region.polygon) {
            self.err(ch, "base_object_modified", Some(reference.into()), format!("{reference} of the base design is at ({}, {}) mm, outside {}: it is read-only", c.x, c.y, region.id));
            return None;
        }
        Some(c)
    }

    fn check_net(&mut self, ch: &str, net: &str) -> bool {
        if let Some(why) = self.protected_nets.get(net).cloned() {
            let ev = why.split(" / ").next().map(|rid| self.rule_evidence(rid)).unwrap_or_default();
            self.err_ev(ch, "protected_net", Some(format!("net {net}")), format!("the net {net} carries a protected pin ({why}): an edit does not join it"), ev);
            return false;
        }
        true
    }

    fn check_change(&mut self, change: &Value, after: &mut Option<Snapshot>) -> ChangeResult {
        let ch = change["id"].as_str().unwrap_or("").to_string();
        let op = change["op"].as_str().unwrap_or("").to_string();
        let region_id = change["region"].as_str().unwrap_or("").to_string();
        let before_n = self.findings.len();
        let mut result = ChangeResult { id: ch.clone(), op: op.clone(), region: region_id.clone(), verdict: "accepted", codes: vec![], authorized_by: None, before: Value::Null, after: Value::Null, evidence: vec![] };
        self.change_inner(&ch, &op, &region_id, change, &mut result, after);
        let codes: Vec<String> = self.findings[before_n..].iter().filter(|f| f.severity == "error").map(|f| f.code.clone()).collect();
        if !codes.is_empty() {
            result.verdict = "rejected";
            result.authorized_by = None;
        }
        result.codes = codes;
        result
    }

    fn change_inner(&mut self, ch: &str, op: &str, region_id: &str, c: &Value, result: &mut ChangeResult, after: &mut Option<Snapshot>) {
        let Some(auth) = self.auth else {
            self.err(ch, "no_base_design", None, "the pack has no base_design: nothing in this design may be edited");
            return;
        };
        let Some(region) = auth.regions.iter().find(|r| r.id == region_id).cloned() else {
            let list = auth.regions.iter().map(|r| r.id.as_str()).collect::<Vec<_>>().join(", ");
            self.err_ev(ch, "region_not_editable", Some(region_id.into()), format!("{region_id} is not an editable region of the base design (editable: {list})"), self.region_evidence(region_id));
            return;
        };
        if !region.allowed_ops.iter().any(|o| o == op) {
            self.err(ch, "op_not_allowed", Some(region_id.into()), format!("{region_id} does not allow {op} (allowed: {})", if region.allowed_ops.is_empty() { "none".to_string() } else { region.allowed_ops.join(", ") }));
            return;
        }
        let uuid = c.get("uuid").and_then(Value::as_str);
        let reference = c.get("ref").and_then(Value::as_str).unwrap_or("").to_string();
        let at = |k: &str| -> Option<Pt> { c.get(k).and_then(Value::as_array).and_then(|a| Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])) };
        match op {
            "add_component" => {
                let block = c["block"].as_str().unwrap_or("");
                let role = c["role"].as_str().unwrap_or("");
                if !region.allowed_blocks.iter().any(|b| b == block) {
                    let recalled = self.draft.get("recall").and_then(Value::as_array).is_some_and(|a| a.iter().any(|r| r["id"].as_str() == Some(block)));
                    let known = self.ir.block(block).is_some();
                    self.err(
                        ch,
                        "unauthorized_block",
                        Some(block.into()),
                        format!(
                            "{block} is not an allowed block of {region_id}{}{}",
                            if known { "" } else { " and the pack does not define it" },
                            if recalled { "; it was found by a search, and a search result does not authorise anything" } else { "" }
                        ),
                    );
                    return;
                }
                let Some(b) = self.ir.block(block) else {
                    self.err(ch, "unauthorized_block", Some(block.into()), format!("the pack has no block {block}"));
                    return;
                };
                let role_def = b.components.iter().find(|k| k.get("role").and_then(Value::as_str) == Some(role));
                let Some(role_def) = role_def else {
                    let roles: Vec<&str> = b.components.iter().filter_map(|k| k.get("role").and_then(Value::as_str)).collect();
                    self.err(ch, "unknown_role", Some(block.into()), format!("{block} has no role '{role}' (roles: {})", roles.join(", ")));
                    return;
                };
                if self.base.is_some_and(|s| s.component(&reference).is_some()) {
                    self.err(ch, "ref_exists", Some(reference.clone()), format!("{reference} already exists in the base design: a draft adds new parts, it does not replace existing ones"));
                    return;
                }
                if self.added.contains_key(&reference) {
                    self.err(ch, "duplicate_ref", Some(reference.clone()), format!("{reference} is added twice in this draft"));
                    return;
                }
                let Some(p) = at("at") else { return };
                if !self.place(ch, &region, p, 0.0, None, &reference) {
                    return;
                }
                let kind = role_def.get("kind").and_then(Value::as_str).unwrap_or("other").to_string();
                result.authorized_by = Some(format!("base_design.editable_regions[{region_id}].allowed_blocks: {block} (role {role})"));
                result.evidence = b.evidence.clone();
                result.before = Value::Null;
                result.after = json!({"ref": reference, "block": block, "role": role, "kind": kind, "value": c.get("value"), "at": p});
                self.added.insert(reference.clone(), Added { region: region_id.to_string() });
                if let Some(s) = after {
                    s.components.push(Component {
                        reference: reference.clone(),
                        value: c.get("value").and_then(Value::as_str).map(str::to_string),
                        footprint: c.get("footprint").and_then(Value::as_str).map(str::to_string),
                        kind: Some(kind),
                        x: p[0],
                        y: p[1],
                        ..Default::default()
                    });
                }
            }
            "move_component" | "remove_component" => {
                let is_move = op == "move_component";
                let to = at("to");
                if let Some(a) = self.added.get(&reference) {
                    if a.region != region_id {
                        let own = a.region.clone();
                        self.err(ch, "crosses_region", Some(reference.clone()), format!("{reference} was added in {own}, the change is for {region_id}"));
                        return;
                    }
                    if is_move {
                        let Some(p) = to else { return };
                        if !self.place(ch, &region, p, 0.0, None, &reference) {
                            return;
                        }
                        result.after = json!({"ref": reference, "at": p});
                        if let Some(s) = after {
                            if let Some(k) = s.components.iter_mut().find(|k| k.reference == reference) {
                                result.before = json!({"ref": reference, "at": [k.x, k.y]});
                                let (dx, dy) = (p[0] - k.x, p[1] - k.y);
                                k.x = p[0];
                                k.y = p[1];
                                for pin in s.pins.iter_mut().filter(|q| q.reference == reference) {
                                    pin.x += dx;
                                    pin.y += dy;
                                }
                            }
                        }
                    } else {
                        self.added.remove(&reference);
                        if let Some(s) = after {
                            s.components.retain(|k| k.reference != reference);
                            s.pins.retain(|k| k.reference != reference);
                        }
                    }
                    result.authorized_by = Some(format!("{reference} was added by this draft in {region_id}; {region_id} allows {op}"));
                    return;
                }
                let Some(comp) = self.touchable(ch, &region, &reference, uuid) else { return };
                if is_move {
                    let Some(p) = to else { return };
                    if !self.place(ch, &region, p, 0.0, None, &reference) {
                        return;
                    }
                    result.before = json!({"ref": reference, "at": [comp.x, comp.y]});
                    result.after = json!({"ref": reference, "at": p});
                    if let Some(s) = after {
                        let (dx, dy) = (p[0] - comp.x, p[1] - comp.y);
                        if let Some(k) = s.components.iter_mut().find(|k| k.reference == reference) {
                            k.x = p[0];
                            k.y = p[1];
                        }
                        for pin in s.pins.iter_mut().filter(|q| q.reference == reference) {
                            pin.x += dx;
                            pin.y += dy;
                        }
                    }
                } else {
                    self.removed.insert(reference.clone());
                    result.before = json!({"ref": reference, "at": [comp.x, comp.y], "value": comp.value});
                    result.after = Value::Null;
                    if let Some(s) = after {
                        s.components.retain(|k| k.reference != reference);
                        s.pins.retain(|k| k.reference != reference);
                    }
                }
                result.authorized_by = Some(format!("{reference} of the base design is inside {region_id}, which allows {op}"));
            }
            "connect" => {
                let pin = c["pin"].as_str().unwrap_or("").to_string();
                let net = c["net"].as_str().unwrap_or("").to_string();
                if !self.check_net(ch, &net) {
                    return;
                }
                let key = (reference.clone(), pin.clone());
                if let Some(a) = self.added.get(&reference) {
                    if a.region != region_id {
                        let own = a.region.clone();
                        self.err(ch, "crosses_region", Some(reference.clone()), format!("{reference} was added in {own}, the connection is for {region_id}"));
                        return;
                    }
                    result.authorized_by = Some(format!("{reference} was added by this draft in {region_id}"));
                    result.before = json!({"ref": reference, "pin": pin, "net": self.connected.get(&key)});
                } else if let Some(dev) = self.ref_device.get(&reference).cloned() {
                    let Some(base) = self.base else {
                        self.err(ch, "base_required", Some(reference.clone()), format!("{reference} is part of the base design: the design is needed to check a connection to its pin"));
                        return;
                    };
                    let Some(comp) = base.component(&reference) else {
                        self.err(ch, "ref_unknown", Some(reference.clone()), format!("{reference} is not in the base design"));
                        return;
                    };
                    if let (Some(want), Some(have)) = (uuid, comp.uuid.as_deref()) {
                        if want != have {
                            self.err(ch, "stale_reference", Some(reference.clone()), format!("{reference} has the identity {have} in the base design, the draft means {want}"));
                            return;
                        }
                    }
                    let label = format!("{reference}.{pin}");
                    if let Some(why) = auth.protected.get(&dev).and_then(|m| m.get(&pin)).cloned() {
                        let ev = self.rule_evidence(&why.rule_id);
                        self.err_ev(ch, "reserved_pin_used", Some(label), format!("pin {pin} of {reference} ({dev}) is protected by {} ({}) and cannot be used by an edit", why.rule_id, why.kind), ev);
                        return;
                    }
                    if auth.immutable_pins.contains(&label) {
                        let msg = format!("{label} is listed as an immutable pin of the base design");
                        self.err(ch, "immutable_object", Some(label), msg);
                        return;
                    }
                    if !region.pin_pool.get(&dev).is_some_and(|p| p.contains(&pin)) {
                        self.err(ch, "pin_not_in_pool", Some(label), format!("pin {pin} of {dev} is not in the pin pool of {region_id}"));
                        return;
                    }
                    if validate::pin_in_use(self.ir, base, &reference, &pin) || self.connected.contains_key(&key) {
                        self.err(ch, "pin_in_use", Some(label), format!("pin {pin} of {reference} is already connected in the base design or by this draft"));
                        return;
                    }
                    result.authorized_by = Some(format!("base_design.editable_regions[{region_id}].pin_pool: {dev} pin {pin}"));
                    result.before = json!({"ref": reference, "pin": pin, "net": base.pin(&reference, &pin).and_then(|p| p.net.clone())});
                } else {
                    let Some(comp) = self.touchable(ch, &region, &reference, uuid) else { return };
                    let Some(base) = self.base else { return };
                    if base.pin(&reference, &pin).is_none() {
                        self.err(ch, "ref_unknown", Some(format!("{reference}.{pin}")), format!("{reference} has no pin {pin} in the base design"));
                        return;
                    }
                    result.authorized_by = Some(format!("{reference} of the base design is inside {region_id}, which allows connect"));
                    result.before = json!({"ref": reference, "pin": pin, "net": base.pin(&reference, &pin).and_then(|p| p.net.clone())});
                    let _ = comp;
                }
                self.connected.insert(key, net.clone());
                result.after = json!({"ref": reference, "pin": pin, "net": net});
                if let Some(s) = after {
                    if let Some(p) = s.pins.iter_mut().find(|p| p.reference == reference && p.pin == pin) {
                        p.net = Some(net.clone());
                    } else if let Some(k) = s.components.iter().find(|k| k.reference == reference).cloned() {
                        s.pins.push(PinUse { reference: reference.clone(), pin: pin.clone(), name: None, net: Some(net.clone()), x: k.x, y: k.y });
                    }
                }
            }
            "disconnect" => {
                let pin = c["pin"].as_str().unwrap_or("").to_string();
                let key = (reference.clone(), pin.clone());
                if self.added.get(&reference).is_some_and(|a| a.region == region_id) || self.connected.contains_key(&key) && self.ref_device.contains_key(&reference) {
                    result.authorized_by = Some(format!("{reference}.{pin} was connected by this draft in {region_id}"));
                    result.before = json!({"ref": reference, "pin": pin, "net": self.connected.get(&key)});
                    self.connected.remove(&key);
                } else {
                    let Some(comp) = self.touchable(ch, &region, &reference, uuid) else { return };
                    let net = self.base.and_then(|b| b.pin(&reference, &pin)).and_then(|p| p.net.clone());
                    let _ = comp;
                    result.authorized_by = Some(format!("{reference} of the base design is inside {region_id}, which allows disconnect"));
                    result.before = json!({"ref": reference, "pin": pin, "net": net});
                }
                result.after = json!({"ref": reference, "pin": pin, "net": Value::Null});
                if let Some(s) = after {
                    if let Some(p) = s.pins.iter_mut().find(|p| p.reference == reference && p.pin == pin) {
                        p.net = None;
                    }
                }
            }
            "add_track" => {
                let net = c["net"].as_str().unwrap_or("").to_string();
                let layer = c["layer"].as_str().unwrap_or("").to_string();
                let width = c["width_mm"].as_f64().unwrap_or(0.0);
                let pts: Vec<Pt> = c["points"].as_array().map(|a| a.iter().filter_map(|p| Some([p.get(0)?.as_f64()?, p.get(1)?.as_f64()?])).collect()).unwrap_or_default();
                if !self.check_net(ch, &net) || !self.check_layer(ch, &region, &layer) {
                    return;
                }
                let n0 = self.findings.len();
                for (i, p) in pts.iter().enumerate() {
                    self.place(ch, &region, *p, width / 2.0, Some(&layer), &format!("track point {}", i + 1));
                }
                // copper that leaves the region is routing across a border, whatever lies beyond it
                for f in self.findings[n0..].iter_mut().filter(|f| f.code == "outside_region") {
                    f.code = "crosses_region".into();
                    f.message = format!("the track leaves {region_id}: {}", f.message);
                }
                for w in pts.windows(2) {
                    // a segment may leave the region between two points that are inside it
                    let crossing = (0..region.polygon.len()).any(|i| geom::segment_segment_distance(w[0], w[1], region.polygon[i], region.polygon[(i + 1) % region.polygon.len()]) + 1e-9 < width / 2.0);
                    if crossing && !self.findings[n0..].iter().any(|f| f.code == "crosses_region" || f.code == "outside_region") {
                        self.err(ch, "crosses_region", Some(format!("net {net}")), format!("the track from ({}, {}) to ({}, {}) mm crosses the border of {region_id}", w[0][0], w[0][1], w[1][0], w[1][1]));
                    }
                    let fixed: Vec<FixedRegion> = auth.fixed_regions.clone();
                    for g in fixed.iter().filter(|g| g.layers.iter().any(|x| x == "all" || *x == layer)) {
                        if geom::segment_polygon_distance(w[0], w[1], &g.polygon) <= width / 2.0 + 1e-9 && !self.findings[n0..].iter().any(|f| f.code == "enters_keepout" && f.message.contains(&g.id)) {
                            self.err_ev(ch, "enters_keepout", Some(format!("net {net}")), format!("the track from ({}, {}) to ({}, {}) mm reaches {} ({}), which is fixed", w[0][0], w[0][1], w[1][0], w[1][1], g.id, g.kind), g.evidence.clone());
                        }
                    }
                }
                if self.findings.len() > n0 {
                    return;
                }
                result.authorized_by = Some(format!("{region_id} allows add_track; the whole track is inside it"));
                result.after = json!({"net": net, "layer": layer, "width_mm": width, "points": pts});
                if let Some(s) = after {
                    for w in pts.windows(2) {
                        s.tracks.push(Track { net: Some(net.clone()), layer: layer.clone(), width_mm: width, x1: w[0][0], y1: w[0][1], x2: w[1][0], y2: w[1][1], uuid: None });
                    }
                }
            }
            "add_via" => {
                let net = c["net"].as_str().unwrap_or("").to_string();
                let d = c["diameter_mm"].as_f64().unwrap_or(0.0);
                let Some(p) = at("at") else { return };
                if !self.check_net(ch, &net) {
                    return;
                }
                if !self.place(ch, &region, p, d / 2.0, None, "via") {
                    return;
                }
                result.authorized_by = Some(format!("{region_id} allows add_via; the via is inside it"));
                result.after = json!({"net": net, "at": p, "diameter_mm": d});
                if let Some(s) = after {
                    s.vias.push(Via { net: Some(net), x: p[0], y: p[1], diameter_mm: d, uuid: None });
                }
            }
            _ => {}
        }
    }

    fn check_layer(&mut self, ch: &str, region: &AuthRegion, layer: &str) -> bool {
        if region.layers.iter().any(|l| l == "all" || l == layer) {
            true
        } else {
            self.err(ch, "layer_not_allowed", Some(layer.into()), format!("{} is editable on {} only, not on {layer}", region.id, region.layers.join(", ")));
            false
        }
    }
}

fn schema_diagnostics(draft: &Value) -> Vec<Diagnostic> {
    let mut out: Vec<Diagnostic> = jsonschema::validate(&draft_schema(), draft)
        .into_iter()
        .map(|e| Diagnostic::error("schema", e.path.clone(), None, e.message.clone()))
        .collect();
    let mut seen = BTreeSet::new();
    for (i, c) in draft.get("changes").and_then(Value::as_array).into_iter().flatten().enumerate() {
        if let Some(id) = c.get("id").and_then(Value::as_str) {
            if !seen.insert(id.to_string()) {
                out.push(Diagnostic::error("duplicate_change_id", format!("$.changes[{i}].id"), Some(id.into()), format!("the change id {id} is used twice")));
            }
        }
    }
    out
}

/// Checks a draft. `Err` (diagnostics) if the draft is not a draft at all (schema); every other problem
/// is a finding of the result. `base` is the design the draft is for; `base_files` the digests of the
/// base files found in the working copy (to see whether the base changed since the pack was made).
pub fn check_edit(ir: &Ir, draft: &Value, base: Option<&Snapshot>, base_files: Option<&[BaseFile]>) -> Result<EditCheck, Vec<Diagnostic>> {
    let diags = schema_diagnostics(draft);
    if !diags.is_empty() {
        return Err(diags);
    }
    if !ir.verify_integrity() {
        return Err(vec![Diagnostic::error("ir_integrity", "$", None, "the Constraint IR does not match its integrity hash: compile the pack again")]);
    }
    let auth = authorization(ir);
    let mut ref_device: BTreeMap<String, String> = BTreeMap::new();
    let mut protected_nets: BTreeMap<String, String> = BTreeMap::new();
    if let Some(b) = base {
        for (dev, refs) in validate::device_instances(ir, b) {
            for r in refs {
                ref_device.insert(r, dev.clone());
            }
        }
        if let Some(a) = &auth {
            for p in &b.pins {
                let Some(dev) = ref_device.get(&p.reference) else { continue };
                let Some(net) = p.net.as_deref().filter(|n| !n.is_empty() && !n.starts_with("unconnected-")) else { continue };
                if validate::net_rail(net).is_some() {
                    continue;
                }
                if let Some(why) = a.protected.get(dev).and_then(|m| m.get(&p.pin)) {
                    protected_nets.entry(net.to_string()).or_insert(format!("{} / {}.{}", why.rule_id, p.reference, p.pin));
                }
            }
            for label in &a.immutable_pins {
                if let Some((r, pin)) = label.split_once('.') {
                    if let Some(net) = b.pin(r, pin).and_then(|p| p.net.as_deref()).filter(|n| !n.is_empty() && validate::net_rail(n).is_none()) {
                        protected_nets.entry(net.to_string()).or_insert(format!("immutable pin {label}"));
                    }
                }
            }
        }
    }
    let mut k = Checker { ir, auth: auth.as_ref(), base, draft, ref_device, protected_nets, added: BTreeMap::new(), removed: BTreeSet::new(), connected: BTreeMap::new(), findings: vec![] };

    // 1. is it the draft for this pack and this base?
    let dp = &draft["pack"];
    if dp["sha256"].as_str() != Some(&ir.pack.sha256) || dp["id"].as_str() != Some(&ir.pack.id) || dp["version"].as_str() != Some(&ir.pack.version) {
        k.push(
            "pack_mismatch",
            "error",
            None,
            Some(ir.pack.id.clone()),
            format!(
                "the draft was made for pack {} {} ({}), this is {} {} ({}): the knowledge pack changed, the draft has to be reviewed or made again",
                dp["id"].as_str().unwrap_or("?"),
                dp["version"].as_str().unwrap_or("?"),
                dp["sha256"].as_str().map(|s| &s[..12.min(s.len())]).unwrap_or("?"),
                ir.pack.id,
                ir.pack.version,
                &ir.pack.sha256[..12]
            ),
            vec![],
        );
    }
    let mut base_verified = false;
    match &auth {
        None => k.push("no_base_design", "error", None, None, "the pack has no base_design: nothing in this design may be edited", vec![]),
        Some(a) => {
            if draft["base"]["id"].as_str() != Some(&a.base_id) || draft["base"]["binding_sha256"].as_str() != Some(&a.binding_sha256) {
                k.push(
                    "base_mismatch",
                    "error",
                    None,
                    Some(a.base_id.clone()),
                    format!("the draft is bound to base design {} ({}), the pack binds {} ({}): the base design changed, the draft has to be reviewed or made again", draft["base"]["id"].as_str().unwrap_or("?"), draft["base"]["binding_sha256"].as_str().map(|s| &s[..12.min(s.len())]).unwrap_or("?"), a.base_id, &a.binding_sha256[..12]),
                    vec![],
                );
            }
            let expected: Vec<(String, String)> = a.binding.get("files").and_then(Value::as_array).map(|f| f.iter().filter_map(|x| Some((x["path"].as_str()?.to_string(), x["sha256"].as_str()?.to_string()))).collect()).unwrap_or_default();
            if let Some(files) = base_files {
                base_verified = true;
                for (path, want) in &expected {
                    match files.iter().find(|f| &f.path == path) {
                        Some(BaseFile { sha256: Some(have), .. }) if have == want => {}
                        Some(BaseFile { sha256: Some(have), .. }) => k.push("base_changed", "error", None, Some(path.clone()), format!("{path} of the base design changed: the pack was made for sha256 {}, the file is {}: the constraints may have drifted, the base has to be reviewed", &want[..12.min(want.len())], &have[..12.min(have.len())]), vec![]),
                        _ => k.push("base_changed", "error", None, Some(path.clone()), format!("{path} of the base design is missing: the pack was made for sha256 {}", &want[..12.min(want.len())]), vec![]),
                    }
                }
            } else if let Some(d) = base.and_then(|b| b.project.source_sha256.as_deref()).filter(|_| !expected.is_empty()) {
                if expected.iter().any(|(_, h)| h == d) {
                    base_verified = true;
                } else {
                    base_verified = true;
                    k.push("base_changed", "error", None, None, format!("the design (sha256 {}) is none of the files the pack was made for ({}): the base design changed, review it", &d[..12.min(d.len())], expected.iter().map(|(p, h)| format!("{p} {}", &h[..12.min(h.len())])).collect::<Vec<_>>().join(", ")), vec![]);
                }
            }
            if !base_verified {
                k.push("base_unverified", "warning", None, None, "the base files were not compared with the digests of the pack (give --base-dir, or a KiCad board that is one of the bound files): a change of the base design cannot be seen", vec![]);
            }
        }
    }

    // 2. the draft cannot grant itself anything
    if let (Some(a), Some(claims)) = (&auth, draft.get("claims")) {
        let mut extra = Vec::new();
        for r in strs(claims.get("regions")) {
            if !a.regions.iter().any(|g| g.id == r) {
                extra.push(format!("region {r}"));
            }
        }
        for b in strs(claims.get("blocks")) {
            if !a.regions.iter().any(|g| g.allowed_blocks.contains(&b)) {
                extra.push(format!("block {b}"));
            }
        }
        for p in claims.get("pins").and_then(Value::as_array).into_iter().flatten() {
            let dev = p["device"].as_str().unwrap_or("");
            for pin in strs(p.get("pins")) {
                if !a.regions.iter().any(|g| g.pin_pool.get(dev).is_some_and(|v| v.contains(&pin))) {
                    extra.push(format!("pin {dev}.{pin}"));
                }
            }
        }
        if !extra.is_empty() {
            k.push("widened_authorization", "error", None, None, format!("the draft claims more than the pack authorises: {}", extra.join(", ")), vec![]);
        }
    }
    // search results are information, not authority
    for r in draft.get("recall").and_then(Value::as_array).into_iter().flatten() {
        if r["kind"].as_str() == Some("block") {
            let id = r["id"].as_str().unwrap_or("");
            if !auth.as_ref().is_some_and(|a| a.regions.iter().any(|g| g.allowed_blocks.iter().any(|b| b == id))) {
                k.push("recall_not_authorized", "warning", None, Some(id.into()), format!("{id} was found by a {} search but the pack does not allow it in any editable region: it is ignored", r["source"].as_str().unwrap_or("")), vec![]);
            }
        }
    }

    // 3. the changes
    let mut after: Option<Snapshot> = base.cloned();
    let changes: Vec<Value> = draft["changes"].as_array().cloned().unwrap_or_default();
    let mut results = Vec::new();
    for c in &changes {
        // a change that is refused is not applied: the later changes are judged on what remains
        let mut trial = after.clone();
        let r = k.check_change(c, &mut trial);
        if r.verdict == "accepted" {
            after = trial;
        }
        results.push(r);
    }
    if let Some(stop) = draft.get("stop") {
        if changes.is_empty() {
            let missing = strs(stop.get("missing"));
            k.push("stopped", "warning", None, None, format!("the generator stopped: {}{}", stop["reason"].as_str().unwrap_or(""), if missing.is_empty() { String::new() } else { format!(" (missing: {})", missing.join("; ")) }), vec![]);
        } else {
            k.push("stop_with_changes", "error", None, None, "the draft says it stopped and still carries changes: a draft either stops or proposes changes", vec![]);
        }
    } else if changes.is_empty() {
        k.push("empty_draft", "warning", None, None, "the draft has no changes", vec![]);
    }

    // 4. the deterministic validator, before and after
    let mut validation = None;
    let mut validator_ran = false;
    if let (Some(b), Some(a)) = (base, after.as_ref()) {
        if let (Ok(v0), Ok(v1)) = (validate::validate(ir, b, "base", None), validate::validate(ir, a, "base + draft", None)) {
            validator_ran = true;
            let key = |f: &validate::Finding| (f.rule_id.clone(), f.object.clone().unwrap_or_default(), f.message.clone());
            let fails = |v: &validate::Validation| -> BTreeSet<(String, String, String)> { v.findings.iter().filter(|f| f.verdict == Verdict::Fail && f.severity == "error").map(key).collect() };
            let (f0, f1) = (fails(&v0), fails(&v1));
            let mut new_failures = Vec::new();
            for f in v1.findings.iter().filter(|f| f.verdict == Verdict::Fail && f.severity == "error" && !f0.contains(&key(f))) {
                new_failures.push(json!({"rule_id": f.rule_id, "object": f.object, "message": f.message}));
                k.push("rule_violation", "error", None, f.object.clone(), format!("after the draft, rule {} fails: {}", f.rule_id, f.message), f.evidence.clone());
            }
            let fixed: Vec<Value> = v0.findings.iter().filter(|f| f.verdict == Verdict::Fail && f.severity == "error" && !f1.contains(&key(f))).map(|f| json!({"rule_id": f.rule_id, "object": f.object, "message": f.message})).collect();
            validation = Some(json!({
                "before": {"result": v0.result, "complete": v0.complete, "summary": v0.summary},
                "after": {"result": v1.result, "complete": v1.complete, "summary": v1.summary},
                "already_failing": f0.len(),
                "new_failures": new_failures,
                "fixed": fixed,
            }));
        }
    }
    if !validator_ran {
        k.push("validator_not_run", "warning", None, None, "the rules of the pack were not run on the result: no design was given, so only the whitelist was checked", vec![]);
    }

    // 5. the verdict
    let errors: Vec<&EditFinding> = k.findings.iter().filter(|f| f.severity == "error").collect();
    let boundary = errors.iter().any(|f| !REVIEW_CODES.contains(&f.code.as_str()));
    let mut review_reasons: Vec<String> = errors.iter().filter(|f| REVIEW_CODES.contains(&f.code.as_str())).map(|f| format!("{}: {}", f.code, f.message)).collect();
    review_reasons.dedup();
    let stopped = k.findings.iter().any(|f| f.code == "stopped");
    let status = if boundary {
        "rejected"
    } else if !review_reasons.is_empty() {
        "needs_review"
    } else if stopped {
        "stopped"
    } else {
        "accepted"
    };
    let gaps = k.findings.iter().any(|f| matches!(f.code.as_str(), "base_unverified" | "validator_not_run"));
    let complete = status == "accepted" && !gaps;
    let acc = results.iter().filter(|r| r.verdict == "accepted").count();
    let count = |op: &str| results.iter().filter(|r| r.verdict == "accepted" && r.op == op).count();
    let diff = json!({
        "components": {
            "added": results.iter().filter(|r| r.verdict == "accepted" && r.op == "add_component").map(|r| r.after.clone()).collect::<Vec<_>>(),
            "moved": results.iter().filter(|r| r.verdict == "accepted" && r.op == "move_component").map(|r| json!({"from": r.before, "to": r.after})).collect::<Vec<_>>(),
            "removed": results.iter().filter(|r| r.verdict == "accepted" && r.op == "remove_component").map(|r| r.before.clone()).collect::<Vec<_>>(),
        },
        "connections": results.iter().filter(|r| r.verdict == "accepted" && matches!(r.op.as_str(), "connect" | "disconnect")).map(|r| json!({"op": r.op, "before": r.before, "after": r.after})).collect::<Vec<_>>(),
        "tracks_added": count("add_track"),
        "vias_added": count("add_via"),
        "not_applied": results.iter().filter(|r| r.verdict == "rejected").map(|r| r.id.clone()).collect::<Vec<_>>(),
    });
    let findings = std::mem::take(&mut k.findings);
    Ok(EditCheck {
        format: CHECK_FORMAT,
        format_version: "1.0",
        fastroute_version: env!("CARGO_PKG_VERSION"),
        status,
        complete,
        pack: json!({"id": ir.pack.id, "version": ir.pack.version, "sha256": ir.pack.sha256, "review": ir.pack.review}),
        base: json!({"id": auth.as_ref().map(|a| a.base_id.clone()), "binding_sha256": auth.as_ref().map(|a| a.binding_sha256.clone()), "verified": base_verified}),
        summary: json!({"changes": results.len(), "accepted": acc, "rejected": results.len() - acc}),
        review_reasons,
        findings,
        changes: results,
        diff,
        validation,
    })
}

// ---- the report -----------------------------------------------------------------------------------

/// What a code means, for the person who reads the report.
pub fn code_meaning(code: &str, lang: Lang) -> &'static str {
    match code {
        "pack_mismatch" => t(lang, "草案引用的知识包版本与当前不同：需要复核", "the draft refers to another version of the knowledge pack: review"),
        "base_mismatch" => t(lang, "草案绑定的基板与知识包绑定的不同：需要复核", "the draft is bound to another base design than the pack: review"),
        "base_changed" => t(lang, "基板文件摘要已变化，约束可能已漂移：需要复核", "the base files changed since the pack was made, the constraints may have drifted: review"),
        "base_unverified" => t(lang, "没有核对基板文件摘要", "the base files were not compared with the pack's digests"),
        "widened_authorization" => t(lang, "草案声明的权限超出知识包白名单", "the draft claims more than the whitelist of the pack"),
        "no_base_design" => t(lang, "知识包没有 base_design：任何东西都不可修改", "the pack has no base_design: nothing may be edited"),
        "region_not_editable" => t(lang, "区域不在可编辑区域白名单内", "the region is not on the editable whitelist"),
        "op_not_allowed" => t(lang, "该区域不允许此操作", "the region does not allow this operation"),
        "unauthorized_block" => t(lang, "外设块未被授权（搜索结果不构成授权）", "the block is not authorised (a search result is not an authorisation)"),
        "unknown_role" => t(lang, "外设块没有这个角色", "the block has no such role"),
        "outside_region" => t(lang, "对象在授权区域之外", "the object is outside the authorised region"),
        "outside_board" => t(lang, "对象在板框之外", "the object is outside the board outline"),
        "enters_keepout" => t(lang, "进入了固定的禁区", "it enters a fixed keep-out"),
        "crosses_region" => t(lang, "跨越了授权区域的边界（跨区域布线）", "it crosses the border of the authorised region"),
        "immutable_object" => t(lang, "固定对象不得修改", "a fixed object may not be changed"),
        "base_object_modified" => t(lang, "试图修改基板上的只读对象", "it changes a read-only object of the base design"),
        "ref_exists" => t(lang, "位号已存在于基板", "the reference already exists in the base design"),
        "duplicate_ref" => t(lang, "草案内位号重复", "the reference is added twice"),
        "ref_unknown" => t(lang, "找不到该对象", "no such object"),
        "stale_reference" => t(lang, "对象标识与基板不符（基板可能已重新编号）", "the object's identity does not match the base design (it may have been renumbered)"),
        "reserved_pin_used" => t(lang, "使用了受保护的引脚（保留、NC、strapping、复位、启动、固定网络）", "it uses a protected pin (reserved, no-connect, strapping, reset, boot, fixed net)"),
        "pin_not_in_pool" => t(lang, "引脚不在该区域的引脚池内", "the pin is not in the region's pin pool"),
        "pin_in_use" => t(lang, "引脚已被基板或本草案使用", "the pin is already used"),
        "protected_net" => t(lang, "网络连着受保护的引脚", "the net carries a protected pin"),
        "layer_not_allowed" => t(lang, "该区域不允许在此层上修改", "the region may not be edited on this layer"),
        "base_required" => t(lang, "检查需要基板设计（--design 或 --kicad-pcb）", "the base design is needed to check this change"),
        "rule_violation" => t(lang, "应用草案后，知识包中的规则不再满足", "after the draft, a rule of the pack is no longer met"),
        "stop_with_changes" => t(lang, "草案声明停止，却带有修改", "the draft stops and still carries changes"),
        "stopped" => t(lang, "生成方停止并报告了缺失的信息", "the generator stopped and reported what is missing"),
        "empty_draft" => t(lang, "草案没有修改", "the draft has no changes"),
        "recall_not_authorized" => t(lang, "搜索召回的外设块不在白名单内，已忽略", "a block found by a search is not on the whitelist: ignored"),
        "validator_not_run" => t(lang, "没有运行知识包规则（没有提供设计）", "the pack's rules were not run (no design given)"),
        _ => "",
    }
}

fn describe(r: &ChangeResult, lang: Lang) -> String {
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let pt = |v: &Value| v.get("at").and_then(Value::as_array).map(|a| format!("({}, {})", a[0], a[1])).unwrap_or_default();
    match r.op.as_str() {
        "add_component" => format!("{} {} ({} / {} {}) @ {} mm", t(lang, "新增", "add"), s(&r.after, "ref"), s(&r.after, "block"), t(lang, "角色", "role"), s(&r.after, "role"), pt(&r.after)),
        "move_component" => format!("{} {}: {} → {} mm", t(lang, "移动", "move"), s(&r.after, "ref"), pt(&r.before), pt(&r.after)),
        "remove_component" => format!("{} {}", t(lang, "删除", "remove"), s(&r.before, "ref")),
        "connect" => format!("{} {}.{}: {} → {}", t(lang, "连接", "connect"), s(&r.after, "ref"), s(&r.after, "pin"), r.before.get("net").and_then(Value::as_str).unwrap_or(t(lang, "（无）", "(none)")), s(&r.after, "net")),
        "disconnect" => format!("{} {}.{} ({})", t(lang, "断开", "disconnect"), s(&r.after, "ref"), s(&r.after, "pin"), r.before.get("net").and_then(Value::as_str).unwrap_or("")),
        "add_track" => format!("{} {} {} {} mm", t(lang, "新增走线", "add track"), s(&r.after, "net"), s(&r.after, "layer"), r.after.get("width_mm").map(|w| w.to_string()).unwrap_or_default()),
        "add_via" => format!("{} {} @ {} mm", t(lang, "新增过孔", "add via"), s(&r.after, "net"), pt(&r.after)),
        _ => r.op.clone(),
    }
}

/// The human-readable report: the verdict, what is bound to what, every change with what authorises
/// it or why it was refused, and the rules before and after.
pub fn edit_report(c: &EditCheck, ir: &Ir, lang: Lang) -> String {
    let mut o = String::new();
    let status = match c.status {
        "accepted" if c.complete => t(lang, "通过（草案在授权范围内）", "accepted (the draft stays inside what is authorised)"),
        "accepted" => t(lang, "通过，但有未检查项", "accepted, with parts not checked"),
        "rejected" => t(lang, "拒绝（有越界修改）", "rejected (changes outside what is authorised)"),
        "needs_review" => t(lang, "需要复核（知识包或基板已变化）", "needs review (the pack or the base design changed)"),
        _ => t(lang, "生成已停止（资料不足或超出能力）", "stopped (missing information or beyond the whitelist)"),
    };
    o.push_str(&format!("# {}\n\n", t(lang, "AI 修改草案校验报告", "AI edit draft check report")));
    o.push_str(&format!("**{}**：{}\n\n", t(lang, "结论", "Result"), status));
    o.push_str(&format!(
        "{}\n\n",
        t(
            lang,
            "草案不会覆盖基板工程：这份报告是唯一的输出，基板文件没有被读写以外的任何操作触及。",
            "The draft does not overwrite the base project: this report is the only output, and the base files are only read."
        )
    ));
    o.push_str(&format!("## 1. {}\n\n", t(lang, "引用的版本", "What it is bound to")));
    o.push_str(&format!("- {}: `{}` {} (sha256 `{}`, {})\n", t(lang, "知识包", "knowledge pack"), c.pack["id"].as_str().unwrap_or(""), c.pack["version"].as_str().unwrap_or(""), &c.pack["sha256"].as_str().unwrap_or("")[..12.min(c.pack["sha256"].as_str().unwrap_or("").len())], c.pack["review"].as_str().unwrap_or("")));
    o.push_str(&format!("- {}: `{}` (binding sha256 `{}`), {}\n\n", t(lang, "基板", "base design"), c.base["id"].as_str().unwrap_or("-"), &c.base["binding_sha256"].as_str().unwrap_or("")[..12.min(c.base["binding_sha256"].as_str().unwrap_or("").len())], if c.base["verified"].as_bool() == Some(true) { t(lang, "文件摘要已核对", "file digests compared") } else { t(lang, "文件摘要未核对", "file digests not compared") }));
    if !c.review_reasons.is_empty() {
        o.push_str(&format!("{}\n\n", t(lang, "需要复核的原因：", "Why a review is needed:")));
        for r in &c.review_reasons {
            o.push_str(&format!("- {r}\n"));
        }
        o.push('\n');
    }
    o.push_str(&format!("## 2. {} ({} / {})\n\n", t(lang, "修改与依据", "Changes and what authorises them"), c.summary["accepted"], c.summary["changes"]));
    if c.changes.is_empty() {
        o.push_str(&format!("{}\n\n", t(lang, "（没有修改）", "(no changes)")));
    } else {
        o.push_str(&format!("| {} | {} | {} | {} |\n|---|---|---|---|\n", t(lang, "编号", "id"), t(lang, "修改", "change"), t(lang, "区域", "region"), t(lang, "判定与依据", "verdict and authority")));
        for r in &c.changes {
            let verdict = if r.verdict == "accepted" {
                format!("✔ {}", r.authorized_by.clone().unwrap_or_default())
            } else {
                format!("✘ {}", r.codes.iter().map(|k| format!("`{k}`")).collect::<Vec<_>>().join(", "))
            };
            o.push_str(&format!("| {} | {} | {} | {} |\n", r.id, describe(r, lang).replace('|', "/"), r.region, verdict.replace('|', "/")));
        }
        o.push('\n');
        for r in c.changes.iter().filter(|r| r.verdict == "accepted" && !r.evidence.is_empty()) {
            o.push_str(&format!("- {}: ", r.id));
            o.push_str(&r.evidence.iter().map(crate::report::evidence_line).collect::<Vec<_>>().join("; "));
            o.push('\n');
        }
        o.push('\n');
    }
    o.push_str(&format!("## 3. {}\n\n", t(lang, "问题", "Findings")));
    if c.findings.is_empty() {
        o.push_str(&format!("{}\n\n", t(lang, "没有问题。", "None.")));
    }
    for f in &c.findings {
        let meaning = code_meaning(&f.code, lang);
        o.push_str(&format!("- **{}** `{}`{}{}: {}\n", if f.severity == "error" { t(lang, "错误", "error") } else { t(lang, "提示", "note") }, f.code, f.change.as_ref().map(|c| format!(" ({c})")).unwrap_or_default(), if meaning.is_empty() { String::new() } else { format!(" — {meaning}") }, f.message));
        for e in &f.evidence {
            o.push_str(&format!("  - {}: {}\n", t(lang, "依据", "evidence"), crate::report::evidence_line(e)));
        }
    }
    o.push('\n');
    o.push_str(&format!("## 4. {}\n\n", t(lang, "修改前后", "Before and after")));
    let d = &c.diff;
    o.push_str(&format!(
        "- {}: +{} / ~{} / −{}\n- {}: {}\n- {}: +{} {}, +{} {}\n",
        t(lang, "元件（新增/移动/删除）", "components (added / moved / removed)"),
        d["components"]["added"].as_array().map(Vec::len).unwrap_or(0),
        d["components"]["moved"].as_array().map(Vec::len).unwrap_or(0),
        d["components"]["removed"].as_array().map(Vec::len).unwrap_or(0),
        t(lang, "连接变化", "connection changes"),
        d["connections"].as_array().map(Vec::len).unwrap_or(0),
        t(lang, "铜", "copper"),
        d["tracks_added"],
        t(lang, "段走线", "track segment(s)"),
        d["vias_added"],
        t(lang, "个过孔", "via(s)"),
    ));
    if let Some(n) = d["not_applied"].as_array().filter(|n| !n.is_empty()) {
        o.push_str(&format!("- {}: {}\n", t(lang, "未应用（被拒绝）", "not applied (refused)"), n.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")));
    }
    o.push('\n');
    match &c.validation {
        Some(v) => {
            let s = |side: &str, k: &str| v[side]["summary"][k].as_u64().unwrap_or(0);
            o.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n|---|---|---|---|---|\n| {} | {} | {} | {} | {} |\n| {} | {} | {} | {} | {} |\n\n",
                t(lang, "知识包规则", "pack rules"),
                t(lang, "通过", "passed"),
                t(lang, "违反", "failed"),
                t(lang, "未检查", "not checked"),
                t(lang, "结果", "result"),
                t(lang, "基板", "base"),
                s("before", "passed"),
                s("before", "failed"),
                s("before", "unchecked") + s("before", "unsupported") + s("before", "unknown") + s("before", "conflict"),
                v["before"]["result"].as_str().unwrap_or(""),
                t(lang, "基板 + 草案", "base + draft"),
                s("after", "passed"),
                s("after", "failed"),
                s("after", "unchecked") + s("after", "unsupported") + s("after", "unknown") + s("after", "conflict"),
                v["after"]["result"].as_str().unwrap_or(""),
            ));
            for n in v["new_failures"].as_array().into_iter().flatten() {
                o.push_str(&format!("- {} **{}**: {}\n", t(lang, "新增违反", "new failure"), n["rule_id"].as_str().unwrap_or(""), n["message"].as_str().unwrap_or("")));
                if let Some(rule) = ir.rule(n["rule_id"].as_str().unwrap_or("")) {
                    for e in &rule.evidence {
                        o.push_str(&format!("  - {}: {}\n", t(lang, "依据", "evidence"), crate::report::evidence_line(e)));
                    }
                }
            }
            for n in v["fixed"].as_array().into_iter().flatten() {
                o.push_str(&format!("- {} {}: {}\n", t(lang, "草案消除了", "fixed by the draft"), n["rule_id"].as_str().unwrap_or(""), n["message"].as_str().unwrap_or("")));
            }
            if v["already_failing"].as_u64().unwrap_or(0) > 0 {
                o.push_str(&format!("- {}: {}\n", t(lang, "基板本身已有的违反（与草案无关）", "failures already in the base design (not caused by the draft)"), v["already_failing"]));
            }
            o.push('\n');
        }
        None => o.push_str(&format!("{}\n\n", t(lang, "没有运行知识包规则（没有提供设计），只检查了白名单。", "The pack's rules were not run (no design given): only the whitelist was checked."))),
    }
    o.push_str(&format!("## 5. {}\n\n", t(lang, "没有检查的内容", "What is not checked")));
    o.push_str(&format!(
        "- {}\n- {}\n",
        t(lang, "走线与基板其余走线的间距、短路：由 KiCad 的 DRC 检查（本工具不替代）。", "Clearance and shorts against the rest of the base design's copper: KiCad's DRC checks them (this tool does not replace it)."),
        t(lang, "电气正确性（元件值、电流）：不在检查范围内。", "Electrical correctness (component values, currents) is outside this check.")
    ));
    o
}
