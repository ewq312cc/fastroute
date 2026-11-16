//! The deterministic validator: a compiled Constraint IR against a design snapshot. It reads only
//! those two (no AI, no network, no vector index, no clock) and gives the same answer every time.
//! Every finding names the rule, the object, the actual and the required value, the severity and
//! the evidence of the rule. What it cannot check is reported as such, never as a pass.

use crate::geom::{self, Pt};
use crate::hcp::sha256_hex;
use crate::ir::{Ir, IrEvidence, IrRegion, IrRule};
use crate::snapshot::{parse_value, Component, Snapshot};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const VALIDATION_FORMAT: &str = "fastroute-hcp-validation";

/// What happened to one rule (or one object of it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// worst first, so that the worst verdict of a rule is the smallest
    Fail,
    Unchecked,
    Unsupported,
    Conflict,
    Unknown,
    NotApplicable,
    Pass,
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub rule_id: String,
    pub kind: String,
    pub verdict: Verdict,
    pub severity: String,
    /// The object of the design: `U1.10`, `net USB_DP`, `block BLK-POWER`, ...
    #[serde(skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required: Option<Value>,
    pub evidence: Vec<IrEvidence>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub rules: usize,
    pub passed: usize,
    pub failed: usize,
    pub not_applicable: usize,
    pub unchecked: usize,
    pub unsupported: usize,
    pub unknown: usize,
    pub conflict: usize,
    /// failing findings that are warnings (they do not fail the result)
    pub warnings: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Validation {
    pub format: &'static str,
    pub format_version: &'static str,
    pub fastroute_version: &'static str,
    pub ir: Value,
    pub design: Value,
    /// `pass` (no failing error) or `fail`
    pub result: &'static str,
    /// Whether every rule was checked: no unchecked, unsupported, unknown or conflicting rule.
    pub complete: bool,
    pub summary: Summary,
    pub findings: Vec<Finding>,
}

struct Ctx<'a> {
    ir: &'a Ir,
    snap: &'a Snapshot,
    /// device id -> its components in the design
    instances: BTreeMap<String, Vec<&'a Component>>,
    device_refs: BTreeSet<&'a str>,
    net_pins: BTreeMap<&'a str, Vec<(&'a str, &'a str)>>,
    copper_nets: BTreeSet<&'a str>,
}

/// `*` and `?` as in KiCad net patterns.
pub fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(&pattern.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

/// A pattern matches the net's full name or, for sheet-qualified names (`/power/3V3`), its last part.
pub fn net_matches(pattern: &str, net: &str) -> bool {
    glob(pattern, net) || (!pattern.starts_with('/') && net.rsplit('/').next().is_some_and(|last| glob(pattern, last)))
}

impl<'a> Ctx<'a> {
    fn new(ir: &'a Ir, snap: &'a Snapshot) -> Self {
        let mut instances: BTreeMap<String, Vec<&Component>> = BTreeMap::new();
        let mut device_refs = BTreeSet::new();
        for c in &snap.components {
            let dev = c.device.clone().or_else(|| {
                let v = c.value.as_deref()?;
                ir.devices.iter().find(|d| d.part_number.eq_ignore_ascii_case(v)).map(|d| d.id.clone())
            });
            if let Some(d) = dev {
                device_refs.insert(c.reference.as_str());
                instances.entry(d).or_default().push(c);
            }
        }
        let mut net_pins: BTreeMap<&str, Vec<(&str, &str)>> = BTreeMap::new();
        for p in &snap.pins {
            if let Some(n) = p.net.as_deref().filter(|n| !n.is_empty()) {
                net_pins.entry(n).or_default().push((p.reference.as_str(), p.pin.as_str()));
            }
        }
        let mut copper_nets = BTreeSet::new();
        for n in snap.tracks.iter().filter_map(|t| t.net.as_deref()).chain(snap.vias.iter().filter_map(|v| v.net.as_deref())).chain(snap.zones.iter().filter(|z| !z.keepout).filter_map(|z| z.net.as_deref())) {
            copper_nets.insert(n);
        }
        Ctx { ir, snap, instances, device_refs, net_pins, copper_nets }
    }

    fn net_of(&self, reference: &str, pin: &str) -> Option<&str> {
        self.snap.pin(reference, pin).and_then(|p| p.net.as_deref()).filter(|n| !n.is_empty())
    }

    /// Whether the pin is connected to anything: a net that is not KiCad's "unconnected" one and
    /// has another pin or copper on it.
    fn pin_used(&self, reference: &str, pin: &str) -> bool {
        match self.net_of(reference, pin) {
            None => false,
            Some(n) if n.starts_with("unconnected-") => false,
            Some(n) => self.net_pins.get(n).is_some_and(|v| v.len() > 1) || self.copper_nets.contains(n),
        }
    }

    fn assignments(&self, reference: &str) -> Vec<(&str, &str)> {
        self.snap.assignments.iter().filter(|a| a.reference == reference).map(|a| (a.pin.as_str(), a.function.as_str())).collect()
    }
}

struct Out<'a> {
    rule: &'a IrRule,
    findings: Vec<Finding>,
}

impl<'a> Out<'a> {
    fn push(&mut self, verdict: Verdict, object: Option<String>, message: impl Into<String>, actual: Option<Value>, required: Option<Value>) {
        self.findings.push(Finding {
            rule_id: self.rule.rule_id.clone(),
            kind: self.rule.kind.clone(),
            verdict,
            severity: self.rule.severity.clone(),
            object,
            message: message.into(),
            actual,
            required,
            evidence: self.rule.evidence.clone(),
        });
    }

    fn fail(&mut self, object: impl Into<String>, message: impl Into<String>, actual: Value, required: Value) {
        self.push(Verdict::Fail, Some(object.into()), message, Some(actual), Some(required));
    }

    fn has_fail(&self) -> bool {
        self.findings.iter().any(|f| f.verdict == Verdict::Fail)
    }

    fn pass_if_clean(&mut self, what: &str) {
        if self.findings.is_empty() {
            self.push(Verdict::Pass, None, what.to_string(), None, None);
        }
    }
}

fn strings(v: &Value, key: &str) -> Vec<String> {
    v.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

fn rule_device<'a>(ctx: &Ctx<'a>, rule: &IrRule) -> Option<String> {
    rule.subject.get("device").and_then(Value::as_str).map(str::to_string).or_else(|| {
        let b = rule.subject.get("block").and_then(Value::as_str)?;
        ctx.ir.block(b)?.device.clone()
    })
}

/// The instances of the rule's device; a finding `not_applicable` if the design has none.
fn instances<'a>(ctx: &Ctx<'a>, rule: &IrRule, out: &mut Out<'_>) -> Vec<&'a Component> {
    let Some(dev) = rule_device(ctx, rule) else { return vec![] };
    match ctx.instances.get(&dev) {
        Some(v) if !v.is_empty() => v.clone(),
        _ => {
            out.push(Verdict::NotApplicable, Some(dev.clone()), format!("the design has no component for {dev} (set `device` on the component, or give it the part number as its value)"), None, None);
            vec![]
        }
    }
}

fn check_reserved(ctx: &Ctx, rule: &IrRule, out: &mut Out, what: &str) {
    for c in instances(ctx, rule, out) {
        for pin in strings(&rule.params, "pins") {
            let assigned: Vec<&str> = ctx.assignments(&c.reference).into_iter().filter(|(p, _)| *p == pin).map(|(_, f)| f).collect();
            if ctx.pin_used(&c.reference, &pin) || !assigned.is_empty() {
                let net = ctx.net_of(&c.reference, &pin).map(str::to_string);
                out.fail(
                    format!("{}.{pin}", c.reference),
                    format!("pin {pin} of {} is {what} but the design uses it{}", c.reference, net.as_ref().map(|n| format!(" (net {n})")).unwrap_or_default()),
                    json!({"net": net, "assigned_functions": assigned}),
                    json!(if what == "reserved" { "not used by the design" } else { "unconnected" }),
                );
            }
        }
    }
}

fn check_fixed_net(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let (pin, pattern) = (rule.params["pin"].as_str().unwrap_or(""), rule.params["net_pattern"].as_str().unwrap_or(""));
    for c in instances(ctx, rule, out) {
        let net = ctx.net_of(&c.reference, pin);
        match net {
            Some(n) if net_matches(pattern, n) => {}
            _ => out.fail(
                format!("{}.{pin}", c.reference),
                match net {
                    Some(n) => format!("pin {pin} of {} is on net {n}, which does not match {pattern}", c.reference),
                    None => format!("pin {pin} of {} is on no net, it must be on a net matching {pattern}", c.reference),
                },
                json!({"net": net}),
                json!({"net_pattern": pattern}),
            ),
        }
    }
}

fn check_exclusive(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let wanted: BTreeSet<String> = strings(&rule.params, "functions").into_iter().collect();
    for c in instances(ctx, rule, out) {
        let a = ctx.assignments(&c.reference);
        if a.is_empty() {
            out.push(Verdict::Unchecked, Some(c.reference.clone()), format!("the design has no pin assignments for {}: the use of functions cannot be checked", c.reference), None, None);
            continue;
        }
        let active: BTreeSet<&str> = a.iter().map(|(_, f)| *f).filter(|f| wanted.contains(*f)).collect();
        if active.len() >= 2 {
            out.fail(
                c.reference.clone(),
                format!("{} uses {} together, which the device does not allow", c.reference, active.iter().cloned().collect::<Vec<_>>().join(" and ")),
                json!({"functions_in_use": active}),
                json!({"at_most_one_of": wanted}),
            );
        }
    }
}

fn check_pin_functions(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let Some(dev) = rule_device(ctx, rule).and_then(|d| ctx.ir.device(&d)) else { return };
    for c in instances(ctx, rule, out) {
        let a = ctx.assignments(&c.reference);
        if a.is_empty() {
            out.push(Verdict::Unchecked, Some(c.reference.clone()), format!("the design has no pin assignments for {}: pin functions cannot be checked", c.reference), None, None);
            continue;
        }
        let mut by_pin: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (p, f) in &a {
            by_pin.entry(p).or_default().insert(f);
        }
        for (p, fs) in by_pin {
            let Some(pin) = dev.pins.iter().find(|x| x.number == p) else {
                out.fail(format!("{}.{p}", c.reference), format!("{} has no pin {p}", dev.id), json!({"pin": p}), json!("a pin of the device"));
                continue;
            };
            for f in &fs {
                if !pin.functions.iter().any(|x| x == f) {
                    out.fail(
                        format!("{}.{p}", c.reference),
                        format!("pin {p} ({}) of {} cannot be {f}; it can be {}", pin.name, c.reference, pin.functions.join(", ")),
                        json!({"function": f}),
                        json!({"one_of": pin.functions}),
                    );
                }
            }
            if fs.len() > 1 {
                out.fail(
                    format!("{}.{p}", c.reference),
                    format!("pin {p} ({}) of {} is given {} functions at once: {}", pin.name, c.reference, fs.len(), fs.iter().cloned().collect::<Vec<_>>().join(", ")),
                    json!({"functions": fs}),
                    json!("one function per pin"),
                );
            }
        }
    }
}

fn component_matches(spec: &Value, c: &Component) -> (bool, Option<String>) {
    let value = c.value.as_deref().and_then(parse_value);
    let readable = |what: &str| (false, Some(format!("{} has no readable {what} ('{}')", c.reference, c.value.clone().unwrap_or_default())));
    if let Some(min) = spec.get("min_value_nf").and_then(Value::as_f64) {
        match value {
            Some(v) if v * 1e9 + 1e-9 >= min => {}
            Some(_) => return (false, None),
            None => return readable("capacitance"),
        }
    }
    if let Some(max) = spec.get("max_value_nf").and_then(Value::as_f64) {
        match value {
            Some(v) if v * 1e9 <= max + 1e-9 => {}
            Some(_) => return (false, None),
            None => return readable("capacitance"),
        }
    }
    if let Some(r) = spec.get("value_ohm").and_then(Value::as_f64) {
        match value {
            Some(v) if (v - r).abs() <= r * 0.05 + 1e-9 => {}
            Some(_) => return (false, None),
            None => return readable("resistance"),
        }
    }
    (true, None)
}

fn check_required_block(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let Some(block) = rule.subject.get("block").and_then(Value::as_str).and_then(|b| ctx.ir.block(b)) else { return };
    if block.connects.is_empty() && !block.components.is_empty() {
        out.push(
            Verdict::Unchecked,
            Some(format!("block {}", block.id)),
            format!("the block {} names no pins it connects to, so its components cannot be found in the design", block.id),
            None,
            None,
        );
        return;
    }
    let devices: Vec<&Component> = if block.device.is_some() { instances(ctx, rule, out) } else { vec![] };
    if block.device.is_some() && devices.is_empty() {
        return;
    }
    for c in devices {
        // the nets the block hangs on
        let mut nets: BTreeSet<&str> = BTreeSet::new();
        for conn in &block.connects {
            let (pin, pattern) = (conn["pin"].as_str().unwrap_or(""), conn["net_pattern"].as_str().unwrap_or(""));
            match ctx.net_of(&c.reference, pin) {
                Some(n) if net_matches(pattern, n) => {
                    nets.insert(n);
                }
                other => {
                    out.fail(
                        format!("{}.{pin}", c.reference),
                        format!("block {}: pin {pin} of {} must be on a net matching {pattern}{}", block.id, c.reference, other.map(|n| format!(", it is on {n}")).unwrap_or_else(|| ", it is on no net".into())),
                        json!({"net": other}),
                        json!({"net_pattern": pattern}),
                    );
                }
            }
        }
        // the components attached to those nets (other than the devices themselves)
        let attached: Vec<&Component> = ctx
            .snap
            .components
            .iter()
            .filter(|k| !ctx.device_refs.contains(k.reference.as_str()))
            .filter(|k| ctx.snap.pins.iter().any(|p| p.reference == k.reference && p.net.as_deref().is_some_and(|n| nets.contains(n))))
            .collect();
        // each component serves one role: the most demanding roles choose first and take the
        // smallest part that satisfies them, so that one large capacitor cannot count twice
        let mut order: Vec<&Value> = block.components.iter().collect();
        order.sort_by(|a, b| {
            let key = |s: &Value| -> f64 {
                ["min_value_nf", "value_ohm"].iter().filter_map(|k| s.get(*k).and_then(Value::as_f64)).fold(-1.0, f64::max)
            };
            key(b).partial_cmp(&key(a)).unwrap_or(std::cmp::Ordering::Equal)
        });
        let mut used: BTreeSet<String> = BTreeSet::new();
        for spec in order {
            let (role, kind) = (spec["role"].as_str().unwrap_or(""), spec["kind"].as_str().unwrap_or(""));
            let need = spec.get("min_count").and_then(Value::as_u64).unwrap_or(1) as usize;
            let mut candidates: Vec<(f64, String)> = Vec::new();
            let mut notes = Vec::new();
            for k in attached.iter().filter(|k| Snapshot::kind_of(k) == kind && !used.contains(&k.reference)) {
                match component_matches(spec, k) {
                    (true, _) => candidates.push((k.value.as_deref().and_then(parse_value).unwrap_or(f64::MAX), k.reference.clone())),
                    (false, Some(n)) => notes.push(n),
                    (false, None) => {}
                }
            }
            candidates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            let found: Vec<String> = candidates.into_iter().take(need).map(|(_, r)| r).collect();
            used.extend(found.iter().cloned());
            if found.len() < need {
                let mut limits = serde_json::Map::new();
                for key in ["min_value_nf", "max_value_nf", "value_ohm"] {
                    if let Some(v) = spec.get(key) {
                        limits.insert(key.to_string(), v.clone());
                    }
                }
                out.fail(
                    format!("block {}", block.id),
                    format!(
                        "block {} ({}): needs {need} {kind} ({role}) on the nets {}, found {}{}",
                        block.id,
                        block.title,
                        nets.iter().cloned().collect::<Vec<_>>().join(", "),
                        found.len(),
                        if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) }
                    ),
                    json!({"found": found, "nets": nets}),
                    json!({"role": role, "kind": kind, "min_count": need, "limits": limits}),
                );
            }
        }
    }
}

/// The polygon(s) of a region in the board frame (one per instance for a region in a device's frame).
fn region_polygons(ctx: &Ctx, region: &IrRegion) -> Vec<Vec<Pt>> {
    let Some(poly) = &region.polygon else { return vec![] };
    if region.frame == "device" {
        let Some(dev) = &region.device else { return vec![] };
        return ctx
            .instances
            .get(dev)
            .map(|v| v.iter().map(|c| poly.iter().map(|p| geom::to_board(*p, [c.x, c.y], c.rotation_deg)).collect()).collect())
            .unwrap_or_default();
    }
    vec![poly.clone()]
}

fn layer_ok(region_layers: &[String], layer: &str) -> bool {
    region_layers.iter().any(|l| l == "all" || l == layer)
}

fn check_keepout(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let Some(region) = rule.subject.get("region").and_then(Value::as_str).and_then(|r| ctx.ir.region(r)) else { return };
    let polys = region_polygons(ctx, region);
    if polys.is_empty() {
        out.push(Verdict::NotApplicable, Some(format!("region {}", region.id)), format!("region {} has no shape in this design (a region of a device that is not in the design?)", region.id), None, None);
        return;
    }
    let forbid: BTreeSet<String> = strings(&rule.params, "forbid").into_iter().collect();
    // group the findings by (what, net) so that one long track is one finding, not a hundred
    let mut hits: BTreeMap<(String, String), (usize, Pt)> = BTreeMap::new();
    let mut note = |what: &str, net: Option<&str>, at: Pt| {
        let e = hits.entry((what.to_string(), net.unwrap_or("").to_string())).or_insert((0, at));
        e.0 += 1;
    };
    for poly in &polys {
        if forbid.contains("track") {
            for t in ctx.snap.tracks.iter().filter(|t| layer_ok(&region.layers, &t.layer)) {
                if geom::segment_polygon_distance([t.x1, t.y1], [t.x2, t.y2], poly) <= t.width_mm / 2.0 {
                    note("track", t.net.as_deref(), [t.x1, t.y1]);
                }
            }
        }
        if forbid.contains("via") {
            for v in &ctx.snap.vias {
                if geom::point_polygon_distance([v.x, v.y], poly) <= v.diameter_mm / 2.0 {
                    note("via", v.net.as_deref(), [v.x, v.y]);
                }
            }
        }
        if forbid.contains("zone") {
            for z in ctx.snap.zones.iter().filter(|z| !z.keepout && layer_ok(&region.layers, &z.layer)) {
                if geom::polygons_overlap(&z.polygon, poly) {
                    note("zone", z.net.as_deref(), z.polygon[0]);
                }
            }
        }
        if forbid.contains("component") {
            for c in &ctx.snap.components {
                // (the device the region belongs to may overlap its own keep-out: that is the module)
                if region.device.as_ref().is_some_and(|d| ctx.instances.get(d).is_some_and(|v| v.iter().any(|i| i.reference == c.reference))) {
                    continue;
                }
                let inside = geom::point_in_polygon([c.x, c.y], poly) || ctx.snap.pins.iter().any(|p| p.reference == c.reference && geom::point_in_polygon([p.x, p.y], poly));
                if inside {
                    note(&format!("component {}", c.reference), None, [c.x, c.y]);
                }
            }
        }
    }
    for ((what, net), (n, at)) in hits {
        let label = if net.is_empty() { what.clone() } else { format!("{what} on net {net}") };
        out.fail(
            format!("region {}", region.id),
            format!("{n} {label} inside the keep-out {} ({}), first at ({:.2}, {:.2}) mm", region.id, region.kind, at[0], at[1]),
            json!({"object": what, "net": if net.is_empty() { Value::Null } else { json!(net) }, "count": n, "first_at_mm": at}),
            json!({"forbidden": forbid}),
        );
    }
}

fn check_trace_width(ctx: &Ctx, rule: &IrRule, out: &mut Out) {
    let min = rule.params["width_mm"].as_f64().unwrap_or(0.0);
    let patterns = strings(&rule.params, "nets");
    let mut worst: BTreeMap<&str, (f64, usize)> = BTreeMap::new();
    for t in &ctx.snap.tracks {
        let Some(net) = t.net.as_deref() else { continue };
        if !patterns.is_empty() && !patterns.iter().any(|p| net_matches(p, net)) {
            continue;
        }
        if t.width_mm + 1e-6 < min {
            let e = worst.entry(net).or_insert((t.width_mm, 0));
            e.0 = e.0.min(t.width_mm);
            e.1 += 1;
        }
    }
    for (net, (w, n)) in worst {
        out.fail(format!("net {net}"), format!("{n} track(s) of net {net} are narrower than {min} mm (down to {w} mm)"), json!({"min_width_mm": w, "tracks": n}), json!({"width_mm": min}));
    }
}

fn verdict_note(rule: &IrRule) -> String {
    rule.execution.reason.clone().unwrap_or_else(|| "not supported".into())
}

/// Checks `ir` against `snap`. Fails only if the IR is not what a compiler wrote.
pub fn validate(ir: &Ir, snap: &Snapshot, design_name: &str, design_sha256: Option<&str>) -> Result<Validation, String> {
    if !ir.verify_integrity() {
        return Err("the Constraint IR does not match its integrity hash: it was edited after compilation; compile the pack again".into());
    }
    let ctx = Ctx::new(ir, snap);
    let mut findings: Vec<Finding> = Vec::new();
    let mut rule_verdicts: Vec<(String, Verdict, bool)> = Vec::new();
    for rule in &ir.rules {
        let mut out = Out { rule, findings: vec![] };
        match rule.status.as_str() {
            "unknown" => out.push(Verdict::Unknown, None, format!("the documents do not say: {}", rule.missing.clone().unwrap_or_default()), None, None),
            "conflict" => out.push(Verdict::Conflict, None, "the documents disagree; a person has to decide before this can be checked".to_string(), rule.conflict.clone(), None),
            _ if rule.execution.validator != "supported" => out.push(Verdict::Unsupported, None, verdict_note(rule), None, None),
            _ => {
                match rule.kind.as_str() {
                    "reserved_pin" => check_reserved(&ctx, rule, &mut out, "reserved"),
                    "nc_pin" => check_reserved(&ctx, rule, &mut out, "no-connect"),
                    "fixed_net" => check_fixed_net(&ctx, rule, &mut out),
                    "exclusive_functions" => check_exclusive(&ctx, rule, &mut out),
                    "pin_functions" => check_pin_functions(&ctx, rule, &mut out),
                    "required_block" => check_required_block(&ctx, rule, &mut out),
                    "keepout" => check_keepout(&ctx, rule, &mut out),
                    "trace_width" => check_trace_width(&ctx, rule, &mut out),
                    other => out.push(Verdict::Unsupported, None, format!("no check for the kind '{other}'"), None, None),
                }
                if !out.has_fail() {
                    out.pass_if_clean("all objects satisfy the rule");
                }
                // parameters that this version does not look at are said, not skipped
                for p in &rule.execution.unchecked_params {
                    out.push(Verdict::Unchecked, None, format!("the parameter '{p}' is not checked"), None, None);
                }
            }
        }
        let worst = out.findings.iter().map(|f| f.verdict).min().unwrap_or(Verdict::Pass);
        let fails_result = out.findings.iter().any(|f| f.verdict == Verdict::Fail && f.severity == "error");
        rule_verdicts.push((rule.rule_id.clone(), worst, fails_result));
        findings.extend(out.findings);
    }
    let count = |v: Verdict| rule_verdicts.iter().filter(|(_, w, _)| *w == v).count();
    let summary = Summary {
        rules: rule_verdicts.len(),
        passed: count(Verdict::Pass),
        failed: count(Verdict::Fail),
        not_applicable: count(Verdict::NotApplicable),
        unchecked: count(Verdict::Unchecked),
        unsupported: count(Verdict::Unsupported),
        unknown: count(Verdict::Unknown),
        conflict: count(Verdict::Conflict),
        warnings: findings.iter().filter(|f| f.verdict == Verdict::Fail && f.severity == "warning").count(),
    };
    let result = if rule_verdicts.iter().any(|(_, _, f)| *f) { "fail" } else { "pass" };
    let complete = summary.unchecked + summary.unsupported + summary.unknown + summary.conflict == 0;
    // (the findings are in the order of the rules, which is the order of the IR: stable)
    Ok(Validation {
        format: VALIDATION_FORMAT,
        format_version: "1.0",
        fastroute_version: env!("CARGO_PKG_VERSION"),
        ir: json!({"pack_id": ir.pack.id, "pack_version": ir.pack.version, "pack_sha256": ir.pack.sha256, "ir_sha256": ir.integrity.sha256, "ir_version": ir.ir_version, "compiler": ir.compiler.version}),
        design: json!({"name": design_name, "sha256": design_sha256.map(str::to_string).unwrap_or_else(|| sha256_hex(serde_json::to_string(snap).unwrap_or_default().as_bytes())), "snapshot_version": snap.snapshot_version}),
        result,
        complete,
        summary,
        findings,
    })
}
