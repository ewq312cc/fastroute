//! Reading a Hardware Constraint Pack (YAML or JSON) and checking it: the schema first, then what a
//! schema cannot say (references, evidence strength, units, the whitelist of editable things).

use crate::diag::Diagnostic;
use crate::jsonschema;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

/// The published schema (the file in the Skill is what this program enforces).
pub const SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/hcp-v1.schema.json");

/// The newest `hcp_version` this program reads: `1.MINOR`.
pub const SUPPORTED_MINOR: u64 = 0;

pub fn schema() -> Value {
    serde_json::from_str(SCHEMA_TEXT).expect("the embedded HCP schema is valid JSON")
}

/// YAML or JSON text -> a JSON value. (A document that starts with `{` is JSON.)
pub fn parse(text: &str) -> Result<Value, String> {
    let t = text.trim_start_matches('\u{feff}');
    if t.trim_start().starts_with('{') {
        return serde_json::from_str(t).map_err(|e| format!("not valid JSON: {e}"));
    }
    let y: serde_yaml::Value = serde_yaml::from_str(t).map_err(|e| format!("not valid YAML: {e}"))?;
    serde_json::to_value(&y).map_err(|e| format!("the YAML cannot be represented as JSON: {e}"))
}

/// The canonical form of a value: keys sorted, no whitespace. Two packs with the same content
/// have the same canonical text, whatever their key order or formatting.
pub fn canonical_json(v: &Value) -> String {
    fn go(v: &Value, out: &mut String) {
        match v {
            Value::Object(m) => {
                let sorted: BTreeMap<&String, &Value> = m.iter().collect();
                out.push('{');
                for (i, (k, v)) in sorted.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(k).unwrap());
                    out.push(':');
                    go(v, out);
                }
                out.push('}');
            }
            Value::Array(a) => {
                out.push('[');
                for (i, v) in a.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    go(v, out);
                }
                out.push(']');
            }
            other => out.push_str(&other.to_string()),
        }
    }
    let mut s = String::new();
    go(v, &mut s);
    s
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The units a parameter key can end in (`width_mm`, `impedance_ohm`, `min_count`).
pub const UNIT_SUFFIXES: &[&str] = &[
    "mm", "um", "mil", "ohm", "kohm", "v", "mv", "a", "ma", "ua", "uf", "nf", "pf", "uh", "nh", "hz", "khz", "mhz", "ghz", "ns", "ps", "us", "ms", "s", "deg",
    "pct", "count", "ratio", "db",
];

fn has_unit(key: &str) -> bool {
    key.rsplit_once('_').is_some_and(|(_, suffix)| UNIT_SUFFIXES.contains(&suffix))
}

/// The kinds of rule that state something about the chip or module itself: a `fixed` or
/// `required` one needs the manufacturer's own words, not what a reference board happens to do.
const CHIP_KINDS: &[&str] = &[
    "reserved_pin",
    "nc_pin",
    "fixed_net",
    "exclusive_functions",
    "required_block",
    "strapping",
    "reset_pin",
    "boot_pin",
    "electrical",
    "high_speed",
    "keepout",
];

fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn items<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn level_is_official(level: &str) -> bool {
    level.starts_with("official_")
}

/// Which object (`C-003`, `BLK-POWER`, ...) a JSON path is in.
/// The pins of a device that a rule protects: reserved and no-connect pins, strapping, reset and boot
/// pins, and pins with a fixed net. A pin on this list is never part of a pin pool that may be edited.
pub fn protected_pins_of(kind: &str, subject: &Value, params: &Value) -> Vec<String> {
    let list = |v: Option<&Value>| v.and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>()).unwrap_or_default();
    let one = |v: Option<&Value>| v.and_then(Value::as_str).map(|s| vec![s.to_string()]).unwrap_or_default();
    match kind {
        "reserved_pin" | "nc_pin" => list(params.get("pins")),
        "strapping" | "reset_pin" | "boot_pin" => one(subject.get("pin")),
        "fixed_net" => one(params.get("pin")),
        _ => vec![],
    }
}

pub fn object_at(pack: &Value, path: &str) -> Option<String> {
    for (section, key) in [("rules", "id"), ("peripheral_blocks", "id"), ("regions", "id"), ("devices", "id"), ("sources", "source_id")] {
        let prefix = format!("$.{section}[");
        if let Some(rest) = path.strip_prefix(&prefix) {
            let idx: usize = rest.split(']').next()?.parse().ok()?;
            return pack.get(section)?.get(idx)?.get(key)?.as_str().map(str::to_string);
        }
    }
    None
}

/// Every problem in a pack: the schema's, then the semantic ones. Empty = the pack is valid.
pub fn check(pack: &Value) -> Vec<Diagnostic> {
    let mut out = Vec::new();

    // the version first: a clear message instead of a mismatch of the constant
    let version = str_at(pack, "hcp_version").unwrap_or("");
    let minor = version.strip_prefix("1.").and_then(|m| m.parse::<u64>().ok());
    let version_ok = minor.is_some_and(|m| m <= SUPPORTED_MINOR);
    if !version.is_empty() && !version_ok {
        out.push(Diagnostic::error(
            "unsupported_version",
            "$.hcp_version",
            None,
            match minor {
                Some(m) => format!("this pack is version 1.{m}; this program reads 1.{SUPPORTED_MINOR}: use a newer fastroute (minor versions only add properties)"),
                None => format!("hcp_version '{version}' is not a version 1.x this program can read"),
            },
        ));
    }

    // vector / retrieval data never belongs in a pack
    vector_fields(pack, "$", &mut out);

    for e in jsonschema::validate(&schema(), pack) {
        if e.path == "$.hcp_version" && !version_ok {
            continue; // said above
        }
        out.push(Diagnostic::error("schema", e.path.clone(), object_at(pack, &e.path), e.message));
    }
    // (the rest relies on the shapes the schema checks; with a broken shape it would only add noise)
    if out.iter().any(|d| d.code == "schema") {
        return sorted(out);
    }
    semantic(pack, &mut out);
    sorted(out)
}

fn sorted(mut out: Vec<Diagnostic>) -> Vec<Diagnostic> {
    out.sort_by(|a, b| (a.severity, &a.path, &a.code, &a.message).cmp(&(b.severity, &b.path, &b.code, &b.message)));
    out.dedup();
    out
}

const VECTOR_KEYS: &[&str] = &["embedding", "embeddings", "vector", "vectors", "similarity", "score", "scores", "rank", "ranking", "distance", "cosine", "retrieval_score"];

fn vector_fields(v: &Value, path: &str, out: &mut Vec<Diagnostic>) {
    match v {
        Value::Object(m) => {
            for (k, child) in m {
                let key = k.trim_start_matches("x_").to_ascii_lowercase();
                let hit = VECTOR_KEYS.contains(&key.as_str()) || key.starts_with("embedding_") || key.starts_with("vector_") || key.starts_with("similarity_");
                if hit {
                    out.push(Diagnostic::error(
                        "vector_field",
                        format!("{path}.{k}"),
                        None,
                        format!("'{k}' looks like retrieval data (embedding, similarity, score, rank): a pack holds facts and evidence references only; vectors live in the rebuildable index of hcp-manager"),
                    ));
                }
                vector_fields(child, &format!("{path}.{k}"), out);
            }
        }
        Value::Array(a) => {
            for (i, child) in a.iter().enumerate() {
                vector_fields(child, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

struct Index<'a> {
    sources: BTreeMap<&'a str, &'a Value>,
    devices: BTreeMap<&'a str, &'a Value>,
    blocks: BTreeMap<&'a str, &'a Value>,
    rules: BTreeMap<&'a str, &'a Value>,
    regions: BTreeMap<&'a str, &'a Value>,
}

fn unique<'a>(pack: &'a Value, section: &str, key: &str, out: &mut Vec<Diagnostic>) -> BTreeMap<&'a str, &'a Value> {
    let mut map = BTreeMap::new();
    for (i, item) in items(pack, section).iter().enumerate() {
        let Some(id) = str_at(item, key) else { continue };
        if map.insert(id, item).is_some() {
            out.push(Diagnostic::error("duplicate_id", format!("$.{section}[{i}].{key}"), Some(id.to_string()), format!("the id '{id}' is used twice in {section}")));
        }
    }
    map
}

fn semantic(pack: &Value, out: &mut Vec<Diagnostic>) {
    let idx = Index {
        sources: unique(pack, "sources", "source_id", out),
        devices: unique(pack, "devices", "id", out),
        blocks: unique(pack, "peripheral_blocks", "id", out),
        rules: unique(pack, "rules", "id", out),
        regions: unique(pack, "regions", "id", out),
    };
    // a document id belongs to one source
    let mut docs = BTreeSet::new();
    for (i, s) in items(pack, "sources").iter().enumerate() {
        if let Some(d) = str_at(s, "document_id") {
            if !docs.insert(d) {
                out.push(Diagnostic::error("duplicate_id", format!("$.sources[{i}].document_id"), str_at(s, "source_id").map(str::to_string), format!("the document id '{d}' is used by two sources")));
            }
        }
    }

    if let Some(review) = pack.get("pack").and_then(|p| p.get("review")) {
        let status = str_at(review, "status").unwrap_or("draft");
        if status != "draft" && (str_at(review, "reviewed_by").is_none() || str_at(review, "reviewed_on").is_none()) {
            out.push(Diagnostic::error("review_incomplete", "$.pack.review", None, "a reviewed or approved pack names who reviewed it and when"));
        }
    }

    devices(pack, &idx, out);
    blocks(pack, &idx, out);
    rules(pack, &idx, out);
    regions(pack, &idx, out);
    base_design(pack, &idx, out);
}

fn pin_numbers<'a>(device: &'a Value) -> BTreeSet<&'a str> {
    items(device, "pins").iter().filter_map(|p| str_at(p, "number")).collect()
}

fn devices(pack: &Value, idx: &Index, out: &mut Vec<Diagnostic>) {
    for (di, d) in items(pack, "devices").iter().enumerate() {
        let id = str_at(d, "id").map(str::to_string);
        let mut numbers = BTreeSet::new();
        let mut names: BTreeMap<&str, usize> = BTreeMap::new();
        for (pi, p) in items(d, "pins").iter().enumerate() {
            let number = str_at(p, "number").unwrap_or("");
            if !numbers.insert(number) {
                out.push(Diagnostic::error("duplicate_pin", format!("$.devices[{di}].pins[{pi}].number"), id.clone(), format!("pin number '{number}' is listed twice")));
            }
            let name = str_at(p, "name").unwrap_or("");
            let class = str_at(p, "class").unwrap_or("");
            if let Some(first) = names.insert(name, pi) {
                if !["ground", "power", "nc", "reserved"].contains(&class) {
                    out.push(Diagnostic::error(
                        "duplicate_pin",
                        format!("$.devices[{di}].pins[{pi}].name"),
                        id.clone(),
                        format!("pin name '{name}' is used by pins[{first}] and pins[{pi}]; only ground, power, nc and reserved pins may share a name"),
                    ));
                }
            }
            evidence_list(pack, idx, p.get("evidence"), &format!("$.devices[{di}].pins[{pi}].evidence"), id.as_deref(), id.as_deref(), out);
        }
    }
}

/// Checks a list of evidence: the sources exist, apply to the device, and nothing is invented.
fn evidence_list(_pack: &Value, idx: &Index, ev: Option<&Value>, path: &str, device: Option<&str>, object: Option<&str>, out: &mut Vec<Diagnostic>) {
    let Some(list) = ev.and_then(Value::as_array) else { return };
    for (i, e) in list.iter().enumerate() {
        let Some(sid) = str_at(e, "source_id") else { continue };
        let p = format!("{path}[{i}]");
        match idx.sources.get(sid) {
            None => out.push(Diagnostic::error("unknown_reference", format!("{p}.source_id"), object.map(str::to_string), format!("the source '{sid}' is not in sources"))),
            Some(src) => {
                if let Some(dev) = device {
                    let applies: Vec<&str> = items(src, "applies_to").iter().filter_map(Value::as_str).collect();
                    if !applies.contains(&dev) {
                        out.push(Diagnostic::error(
                            "source_not_applicable",
                            format!("{p}.source_id"),
                            object.map(str::to_string),
                            format!("the source '{sid}' applies to {} but the evidence is used for {dev}: a document of another model or version is not evidence for this one", applies.join(", ")),
                        ));
                    }
                }
            }
        }
    }
}

fn device_of_block<'a>(idx: &Index<'a>, block: &str) -> Option<&'a str> {
    idx.blocks.get(block).and_then(|b| str_at(b, "device"))
}

fn blocks(pack: &Value, idx: &Index, out: &mut Vec<Diagnostic>) {
    for (bi, b) in items(pack, "peripheral_blocks").iter().enumerate() {
        let id = str_at(b, "id").unwrap_or("");
        let path = format!("$.peripheral_blocks[{bi}]");
        let device = str_at(b, "device");
        if let Some(dev) = device {
            match idx.devices.get(dev) {
                None => out.push(Diagnostic::error("unknown_reference", format!("{path}.device"), Some(id.into()), format!("the device '{dev}' is not in devices"))),
                Some(d) => {
                    let pins = pin_numbers(d);
                    for (ci, c) in items(b, "connects").iter().enumerate() {
                        let pin = str_at(c, "pin").unwrap_or("");
                        if !pins.contains(pin) {
                            out.push(Diagnostic::error("unknown_pin", format!("{path}.connects[{ci}].pin"), Some(id.into()), format!("{dev} has no pin '{pin}'")));
                        }
                    }
                }
            }
        } else if !items(b, "connects").is_empty() {
            out.push(Diagnostic::error("unknown_reference", format!("{path}.connects"), Some(id.into()), "a block with connections names its device"));
        }
        evidence_list(pack, idx, b.get("evidence"), &format!("{path}.evidence"), device, Some(id), out);
        strength(b, &format!("{path}.evidence"), id, true, idx, out);
    }
}

/// `fixed` / `required` statements about the chip need the manufacturer's words.
fn strength(item: &Value, path: &str, id: &str, chip_level: bool, idx: &Index, out: &mut Vec<Diagnostic>) {
    let modifiability = str_at(item, "modifiability").unwrap_or("");
    if !(chip_level && (modifiability == "fixed" || modifiability == "required")) {
        return;
    }
    let ok = items(item, "evidence").iter().any(|e| {
        str_at(e, "basis") == Some("explicit") && str_at(e, "source_id").and_then(|s| idx.sources.get(s)).and_then(|s| str_at(s, "level")).is_some_and(level_is_official)
    });
    if !ok {
        out.push(Diagnostic::error(
            "weak_evidence",
            path,
            Some(id.to_string()),
            format!("'{id}' is {modifiability}: that needs evidence quoted from an official source (basis explicit); a reference board, a community post, a derivation or a pending note does not make the chip require something"),
        ));
    }
}

fn check_params(params: &Value, path: &str, id: &str, unknown_or_conflict: bool, out: &mut Vec<Diagnostic>) {
    match params {
        Value::Object(m) => {
            for (k, v) in m {
                let p = format!("{path}.{k}");
                if v.is_number() {
                    if unknown_or_conflict {
                        out.push(Diagnostic::error("unknown_value", &p, Some(id.into()), format!("'{k}' has a number although the rule is unknown or in conflict: write null, never a guess")));
                    } else if !has_unit(k) {
                        out.push(Diagnostic::error(
                            "unknown_unit",
                            &p,
                            Some(id.into()),
                            format!("the number '{k}' has no unit: end the key with one of {} (e.g. width_mm)", UNIT_SUFFIXES.join(", ")),
                        ));
                    }
                } else {
                    check_params(v, &p, id, unknown_or_conflict, out);
                }
            }
        }
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                if v.is_number() && !unknown_or_conflict {
                    // (a list of numbers has no key to carry a unit: use named parameters)
                    out.push(Diagnostic::error("unknown_unit", format!("{path}[{i}]"), Some(id.into()), "a bare number in a list has no unit: use named parameters whose keys end in a unit"));
                } else {
                    check_params(v, &format!("{path}[{i}]"), id, unknown_or_conflict, out);
                }
            }
        }
        _ => {}
    }
}

fn rules(pack: &Value, idx: &Index, out: &mut Vec<Diagnostic>) {
    for (ri, r) in items(pack, "rules").iter().enumerate() {
        let id = str_at(r, "id").unwrap_or("");
        let path = format!("$.rules[{ri}]");
        let kind = str_at(r, "kind").unwrap_or("");
        let modifiability = str_at(r, "modifiability").unwrap_or("");
        let subject = r.get("subject");
        let sub = |k: &str| subject.and_then(|s| str_at(s, k));

        // references of the subject
        let mut device = sub("device");
        if let Some(d) = device {
            if !idx.devices.contains_key(d) {
                out.push(Diagnostic::error("unknown_reference", format!("{path}.subject.device"), Some(id.into()), format!("the device '{d}' is not in devices")));
            }
        }
        if let Some(b) = sub("block") {
            if !idx.blocks.contains_key(b) {
                out.push(Diagnostic::error("unknown_reference", format!("{path}.subject.block"), Some(id.into()), format!("the block '{b}' is not in peripheral_blocks")));
            } else if device.is_none() {
                device = device_of_block(idx, b);
            }
        }
        let region = sub("region").and_then(|reg| {
            let found = idx.regions.get(reg).copied();
            if found.is_none() {
                out.push(Diagnostic::error("unknown_reference", format!("{path}.subject.region"), Some(id.into()), format!("the region '{reg}' is not in regions")));
            }
            found
        });
        if let (Some(d), Some(pin)) = (device.and_then(|d| idx.devices.get(d)), sub("pin")) {
            if !pin_numbers(d).contains(pin) {
                out.push(Diagnostic::error("unknown_pin", format!("{path}.subject.pin"), Some(id.into()), format!("{} has no pin '{pin}'", device.unwrap_or(""))));
            }
        }
        // pins named in the parameters
        if let (Some(d), Some(params)) = (device.and_then(|d| idx.devices.get(d)), r.get("params")) {
            let pins = pin_numbers(d);
            let mut named: Vec<(&str, String)> = Vec::new();
            if let Some(a) = params.get("pins").and_then(Value::as_array) {
                named.extend(a.iter().filter_map(Value::as_str).map(|p| (p, format!("{path}.params.pins"))));
            }
            if let Some(p) = params.get("pin").and_then(Value::as_str) {
                named.push((p, format!("{path}.params.pin")));
            }
            for (p, at) in named {
                if !pins.contains(p) {
                    out.push(Diagnostic::error("unknown_pin", at, Some(id.into()), format!("{} has no pin '{p}'", device.unwrap_or(""))));
                }
            }
        }
        if let Some(params) = r.get("params") {
            let weak = modifiability == "unknown" || modifiability == "conflict";
            check_params(params, &format!("{path}.params"), id, weak, out);
        }
        // kind-specific references
        match kind {
            "required_block" => {}
            "keepout" => {
                if let Some(reg) = region {
                    let k = str_at(reg, "kind").unwrap_or("");
                    if !["antenna_keepout", "keepout", "module_body"].contains(&k) {
                        out.push(Diagnostic::error("wrong_region_kind", format!("{path}.subject.region"), Some(id.into()), format!("a keepout rule needs a keepout region, '{}' is a {k}", str_at(reg, "id").unwrap_or(""))));
                    }
                }
            }
            "mechanical_fixed" => {
                if let Some(reg) = region {
                    let k = str_at(reg, "kind").unwrap_or("");
                    if !["mount_hole", "connector", "board_outline", "module_body"].contains(&k) {
                        out.push(Diagnostic::error("wrong_region_kind", format!("{path}.subject.region"), Some(id.into()), format!("a mechanical_fixed rule needs a mount hole, connector, outline or module body, '{}' is a {k}", str_at(reg, "id").unwrap_or(""))));
                    }
                }
            }
            _ => {}
        }

        // evidence
        evidence_list(pack, idx, r.get("evidence"), &format!("{path}.evidence"), device, Some(id), out);
        let chip_level = CHIP_KINDS.contains(&kind);
        strength(r, &format!("{path}.evidence"), id, chip_level, idx, out);
        if modifiability != "unknown" {
            for (ei, e) in items(r, "evidence").iter().enumerate() {
                if str_at(e, "basis") == Some("pending") {
                    out.push(Diagnostic::warning("pending_evidence", format!("{path}.evidence[{ei}]"), Some(id.into()), "evidence that is still pending (to be confirmed) does not support a rule; confirm it or mark the rule unknown"));
                }
            }
        }
        match modifiability {
            "derived" => {
                for (ei, e) in items(r, "evidence").iter().enumerate() {
                    if str_at(e, "basis") != Some("derived") {
                        out.push(Diagnostic::error("derived_basis", format!("{path}.evidence[{ei}].basis"), Some(id.into()), "the evidence of a derived rule says so (basis derived): a derivation is not a quotation"));
                    }
                }
                for (di, dep) in items(r, "derived_from").iter().filter_map(Value::as_str).enumerate() {
                    if dep == id {
                        out.push(Diagnostic::error("derived_cycle", format!("{path}.derived_from[{di}]"), Some(id.into()), "a rule cannot be derived from itself"));
                    } else if !idx.rules.contains_key(dep) {
                        out.push(Diagnostic::error("unknown_reference", format!("{path}.derived_from[{di}]"), Some(id.into()), format!("the rule '{dep}' does not exist")));
                    }
                }
            }
            "conflict" => {
                let cands = r.get("conflict").map(|c| items(c, "candidates")).unwrap_or(&[]);
                let distinct: BTreeSet<String> = cands.iter().filter_map(|c| c.get("value")).map(canonical_json).collect();
                if distinct.len() < 2 {
                    out.push(Diagnostic::error("conflict_state", format!("{path}.conflict.candidates"), Some(id.into()), "a conflict has at least two different values, each with its own evidence"));
                }
                for (ci, c) in cands.iter().enumerate() {
                    evidence_list(pack, idx, c.get("evidence"), &format!("{path}.conflict.candidates[{ci}].evidence"), device, Some(id), out);
                }
            }
            _ => {}
        }
    }
}

fn polygon_area(poly: &[Value]) -> f64 {
    let pts: Vec<(f64, f64)> = poly.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
    let mut a = 0.0;
    for i in 0..pts.len() {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % pts.len()];
        a += x1 * y2 - x2 * y1;
    }
    a / 2.0
}

fn regions(pack: &Value, idx: &Index, out: &mut Vec<Diagnostic>) {
    let mut outlines = 0;
    for (gi, g) in items(pack, "regions").iter().enumerate() {
        let id = str_at(g, "id").unwrap_or("");
        let path = format!("$.regions[{gi}]");
        let kind = str_at(g, "kind").unwrap_or("");
        if let Some(poly) = g.get("polygon").and_then(Value::as_array) {
            if polygon_area(poly).abs() < 1e-9 {
                out.push(Diagnostic::error("degenerate_region", format!("{path}.polygon"), Some(id.into()), "the polygon has no area"));
            }
        }
        if kind == "mount_hole" && g.get("circle").is_none() {
            out.push(Diagnostic::error("wrong_region_kind", format!("{path}.circle"), Some(id.into()), "a mount hole is a circle (center and radius)"));
        }
        if kind == "board_outline" {
            outlines += 1;
            if g.get("polygon").is_none() {
                out.push(Diagnostic::error("wrong_region_kind", format!("{path}.polygon"), Some(id.into()), "the board outline is a polygon"));
            }
        }
        if kind == "board_outline" && outlines > 1 {
            out.push(Diagnostic::error("duplicate_id", &path, Some(id.into()), "there is one board outline"));
        }
        let device = str_at(g, "device");
        if let Some(d) = device {
            if !idx.devices.contains_key(d) {
                out.push(Diagnostic::error("unknown_reference", format!("{path}.device"), Some(id.into()), format!("the device '{d}' is not in devices")));
            }
        }
        if str_at(g, "frame") == Some("device") && device.is_none() {
            out.push(Diagnostic::error("unknown_reference", format!("{path}.device"), Some(id.into()), "a region in the frame of a device names the device"));
        }
        evidence_list(pack, idx, g.get("evidence"), &format!("{path}.evidence"), device, Some(id), out);
    }
}

fn base_design(pack: &Value, idx: &Index, out: &mut Vec<Diagnostic>) {
    let base = pack.get("base_design");
    // the whitelist of what an AI may edit: nothing is editable unless it is listed here
    let mut editable_regions: BTreeSet<&str> = BTreeSet::new();
    let mut allowed_blocks: BTreeSet<&str> = BTreeSet::new();
    if let Some(b) = base {
        for (ei, e) in items(b, "editable_regions").iter().enumerate() {
            let path = format!("$.base_design.editable_regions[{ei}]");
            let reg = str_at(e, "region").unwrap_or("");
            match idx.regions.get(reg) {
                None => out.push(Diagnostic::error("unknown_reference", format!("{path}.region"), None, format!("the region '{reg}' is not in regions"))),
                Some(g) => {
                    editable_regions.insert(reg);
                    if str_at(g, "kind") != Some("editable") {
                        out.push(Diagnostic::error("wrong_region_kind", format!("{path}.region"), Some(reg.into()), format!("an editable region is a region of kind editable, '{reg}' is a {}", str_at(g, "kind").unwrap_or(""))));
                    }
                    if str_at(g, "modifiability") != Some("ai_editable") {
                        out.push(Diagnostic::error("editable_not_whitelisted", format!("{path}.region"), Some(reg.into()), format!("'{reg}' is listed as editable but its modifiability is not ai_editable")));
                    }
                }
            }
            if items(e, "allowed_ops").is_empty() {
                out.push(Diagnostic::warning("no_allowed_ops", format!("{path}.allowed_ops"), Some(reg.into()), "no allowed_ops: nothing may be changed in this region until the operations are listed"));
            }
            for (bi, blk) in items(e, "allowed_blocks").iter().filter_map(Value::as_str).enumerate() {
                if idx.blocks.contains_key(blk) {
                    allowed_blocks.insert(blk);
                } else {
                    out.push(Diagnostic::error("unknown_reference", format!("{path}.allowed_blocks[{bi}]"), None, format!("the block '{blk}' is not in peripheral_blocks")));
                }
            }
            for (pi, pool) in items(e, "pin_pool").iter().enumerate() {
                let dev = str_at(pool, "device").unwrap_or("");
                match idx.devices.get(dev) {
                    None => out.push(Diagnostic::error("unknown_reference", format!("{path}.pin_pool[{pi}].device"), None, format!("the device '{dev}' is not in devices"))),
                    Some(d) => {
                        let pins = pin_numbers(d);
                        for (qi, pin) in items(pool, "pins").iter().filter_map(Value::as_str).enumerate() {
                            if !pins.contains(pin) {
                                out.push(Diagnostic::error("unknown_pin", format!("{path}.pin_pool[{pi}].pins[{qi}]"), None, format!("{dev} has no pin '{pin}'")));
                            }
                            // a protected pin is never handed to an editor, whatever the pool says
                            for r in items(pack, "rules") {
                                if str_at(r.get("subject").unwrap_or(&Value::Null), "device") == Some(dev)
                                    && protected_pins_of(str_at(r, "kind").unwrap_or(""), r.get("subject").unwrap_or(&Value::Null), r.get("params").unwrap_or(&Value::Null)).iter().any(|p| p == pin)
                                {
                                    out.push(Diagnostic::error(
                                        "pool_pin_protected",
                                        format!("{path}.pin_pool[{pi}].pins[{qi}]"),
                                        str_at(r, "id").map(str::to_string),
                                        format!("{dev} pin {pin} is protected by rule {} ({}): it cannot be in a pin pool", str_at(r, "id").unwrap_or(""), str_at(r, "kind").unwrap_or("")),
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        for (ri, reg) in items(b.get("immutable").unwrap_or(&Value::Null), "regions").iter().filter_map(Value::as_str).enumerate() {
            if !idx.regions.contains_key(reg) {
                out.push(Diagnostic::error("unknown_reference", format!("$.base_design.immutable.regions[{ri}]"), None, format!("the region '{reg}' is not in regions")));
            }
            if editable_regions.contains(reg) {
                out.push(Diagnostic::error("editable_not_whitelisted", format!("$.base_design.immutable.regions[{ri}]"), Some(reg.into()), format!("'{reg}' is both immutable and editable")));
            }
        }
    }
    // everything that is ai_editable must be on the whitelist, and nothing is without a base design
    for (gi, g) in items(pack, "regions").iter().enumerate() {
        if str_at(g, "modifiability") == Some("ai_editable") {
            let id = str_at(g, "id").unwrap_or("");
            if !editable_regions.contains(id) {
                out.push(Diagnostic::error(
                    "editable_not_whitelisted",
                    format!("$.regions[{gi}].modifiability"),
                    Some(id.into()),
                    format!("'{id}' is ai_editable but not listed in base_design.editable_regions: everything is read-only unless it is on that list"),
                ));
            }
        }
    }
    for (bi, b) in items(pack, "peripheral_blocks").iter().enumerate() {
        if str_at(b, "modifiability") == Some("ai_editable") {
            let id = str_at(b, "id").unwrap_or("");
            if !allowed_blocks.contains(id) {
                out.push(Diagnostic::error(
                    "editable_not_whitelisted",
                    format!("$.peripheral_blocks[{bi}].modifiability"),
                    Some(id.into()),
                    format!("'{id}' is ai_editable but no editable region allows it (allowed_blocks)"),
                ));
            }
        }
    }
    for (ri, r) in items(pack, "rules").iter().enumerate() {
        if str_at(r, "modifiability") == Some("ai_editable") && base.is_none() {
            out.push(Diagnostic::error("editable_not_whitelisted", format!("$.rules[{ri}].modifiability"), str_at(r, "id").map(str::to_string), "ai_editable needs a base_design with editable_regions"));
        }
    }
}
