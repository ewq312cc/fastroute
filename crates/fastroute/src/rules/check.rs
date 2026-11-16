//! The deterministic checker of a rule draft. No AI, no network: the same draft and design give
//! the same verdict. It enforces what makes a draft trustworthy:
//!
//! * the structure (the published schema, enforced here),
//! * **no routing geometry** anywhere in a draft,
//! * every rule points at words in one of the inputs (quotes are found verbatim), at design data
//!   that exists, or at a reference board,
//! * a number that cannot be read off a board (impedance, current, timing) is only accepted with
//!   a person's words behind it - otherwise the rule has to be `unknown`,
//! * a rule the engine cannot enforce cannot be `hard` or `soft`: it is `unsupported`,
//! * `unknown` rules carry no value,
//! * **a rule may not loosen what the design already demands** (clearance, trace width,
//!   maximum length) unless it says so (`relaxes`) with the design's real baseline, and a person
//!   accepts it explicitly before anything is applied.

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::Serialize;
use serde_json::Value;

use super::facts::{DesignFacts, Scope};
use super::kinds::{self, Capability, Kind, GEOMETRY_KEYS};

pub const FORMAT: &str = "fastroute-rule-check";

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub code: &'static str,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct RuleVerdict {
    pub id: String,
    pub kind: String,
    pub strength: String,
    pub capability: Capability,
    /// Can `rules apply` execute it (hard or soft, and the engine can enforce it).
    pub executable: bool,
    /// Why it is or is not executable.
    pub reason: String,
    /// It loosens a limit of the design (and says so).
    pub relaxes: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Summary {
    pub rules: usize,
    pub hard: usize,
    pub soft: usize,
    pub unknown: usize,
    pub unsupported: usize,
    pub executable: usize,
    pub relaxations: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CheckReport {
    pub format: &'static str,
    pub format_version: &'static str,
    pub draft_sha256: String,
    pub valid: bool,
    /// Whether the rules could be checked against the design (nets, layers, baselines).
    pub design_checked: bool,
    pub errors: Vec<Finding>,
    pub warnings: Vec<Finding>,
    pub rules: Vec<RuleVerdict>,
    pub summary: Summary,
}

struct Ctx<'a> {
    facts: Option<&'a DesignFacts>,
    errors: Vec<Finding>,
    warnings: Vec<Finding>,
}

impl Ctx<'_> {
    fn err(&mut self, code: &'static str, path: &str, message: impl Into<String>) {
        self.errors.push(Finding { code, path: path.to_string(), message: message.into() });
    }
    fn warn(&mut self, code: &'static str, path: &str, message: impl Into<String>) {
        self.warnings.push(Finding { code, path: path.to_string(), message: message.into() });
    }
}

fn norm(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// The numbers written in `s`, in order (`0.15 mm`, `90 ohm`, `3A`, `.5`).
pub fn numbers_in(s: &str) -> Vec<f64> {
    let b: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let digit_at = |k: usize| b.get(k).is_some_and(char::is_ascii_digit);
        // a number starts at a digit, or at a '.' before a digit that does not follow a word
        let starts = digit_at(i) || (b[i] == '.' && digit_at(i + 1) && (i == 0 || !b[i - 1].is_alphanumeric()));
        if !starts {
            i += 1;
            continue;
        }
        let mut j = i;
        let mut dot = false;
        while j < b.len() && (b[j].is_ascii_digit() || (b[j] == '.' && !dot && digit_at(j + 1))) {
            dot |= b[j] == '.';
            j += 1;
        }
        if let Ok(v) = b[i..j].iter().collect::<String>().parse::<f64>() {
            out.push(v);
        }
        i = j;
    }
    out
}

fn scan_geometry(v: &Value, path: &str, ctx: &mut Ctx) {
    match v {
        Value::Object(o) => {
            for (k, child) in o {
                let p = format!("{path}.{k}");
                // `inputs[].path` is the file an input was read from (a property of the schema),
                // the only place where the word is not geometry
                let is_input_file = k == "path" && child.is_string() && path.starts_with("$.inputs[") && path.matches('.').count() == 1;
                if GEOMETRY_KEYS.contains(&k.to_ascii_lowercase().as_str()) && !is_input_file {
                    ctx.err("geometry_in_draft", &p, format!("'{k}' looks like routing geometry; a draft contains rules, never coordinates, traces or vias (the engine does not accept routing from a draft)"));
                }
                // free text is not scanned
                if k != "text" && k != "quote" && k != "rationale" && k != "reason" {
                    scan_geometry(child, &p, ctx);
                }
            }
        }
        Value::Array(a) => a.iter().enumerate().for_each(|(i, c)| scan_geometry(c, &format!("{path}[{i}]"), ctx)),
        _ => {}
    }
}

/// How a rule's scope reads.
fn scope_of(rule: &Value) -> Result<Scope, String> {
    let at = &rule["applies_to"];
    let nets: Vec<String> = at["nets"].as_array().map(|a| a.iter().filter_map(|n| n.as_str().map(str::to_string)).collect()).unwrap_or_default();
    let class = at["net_class"].as_str().map(str::to_string);
    let all = at["all"].as_bool() == Some(true);
    match (!nets.is_empty(), class, all) {
        (true, None, false) => Ok(Scope::Nets(nets)),
        (false, Some(c), false) => Ok(Scope::Class(c)),
        (false, None, true) => Ok(Scope::All),
        _ => Err("applies_to needs exactly one of nets, net_class, all=true".into()),
    }
}

/// The design's current limit for a rule and whether the rule loosens it:
/// `(parameter, baseline, loosens)`.
pub fn baseline(kind: &str, params: &Value, scope: &Scope, facts: &DesignFacts) -> Option<(&'static str, f64, bool)> {
    let classes = facts.classes_of(scope);
    let layer = params["layer"].as_str();
    let pick = |v: &[(String, f64)]| -> Vec<f64> { v.iter().filter(|(l, _)| layer.is_none_or(|w| w == l)).map(|(_, x)| *x).collect() };
    let eps = 1e-9;
    match kind {
        "clearance" => {
            let new = params["clearance_mm"].as_f64()?;
            let base = classes.iter().flat_map(|c| pick(&c.clearance_mm)).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x))))?;
            Some(("clearance_mm", base, new < base - eps))
        }
        "trace_width" => {
            let new = params["width_mm"].as_f64()?;
            let base = classes.iter().flat_map(|c| pick(&c.trace_width_mm)).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.max(x))))?;
            Some(("width_mm", base, new < base - eps))
        }
        "length_max" => {
            let new = params["max_mm"].as_f64()?;
            let base = classes.iter().filter_map(|c| c.max_length_mm).fold(None, |a: Option<f64>, x| Some(a.map_or(x, |a| a.min(x))))?;
            Some(("max_mm", base, new > base + eps))
        }
        _ => None,
    }
}

fn check_param_types(kind: &Kind, params: &Value, path: &str, ctx: &mut Ctx) {
    let Some(obj) = params.as_object() else { return };
    for (k, v) in obj {
        let p = format!("{path}.{k}");
        let Some(def) = kind.params.iter().find(|d| d.key == k) else {
            ctx.err("unknown_param", &p, format!("'{k}' is not a parameter of a {} rule (known: {})", kind.name, kind.params.iter().map(|d| d.key).collect::<Vec<_>>().join(", ")));
            continue;
        };
        match def.ty {
            "number" => match v.as_f64() {
                Some(x) if x.is_finite() && x > 0.0 => {}
                _ => ctx.err("bad_param", &p, format!("'{k}' must be a positive number, not {v}")),
            },
            "string" => {
                if !v.as_str().is_some_and(|s| !s.trim().is_empty()) {
                    ctx.err("bad_param", &p, format!("'{k}' must be a non-empty string, not {v}"));
                }
            }
            "strings" => {
                if !v.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(|s| !s.trim().is_empty()))) {
                    ctx.err("bad_param", &p, format!("'{k}' must be a non-empty list of names, not {v}"));
                }
            }
            "bool" if !v.is_boolean() => ctx.err("bad_param", &p, format!("'{k}' must be true or false, not {v}")),
            _ => {}
        }
        if let Some(f) = ctx.facts {
            let layers: Vec<&str> = match k.as_str() {
                "layer" => v.as_str().into_iter().collect(),
                "allowed_layers" | "preferred_layers" | "avoid_layers" | "layers" => v.as_array().into_iter().flatten().filter_map(Value::as_str).collect(),
                _ => vec![],
            };
            for l in layers {
                if !f.has_layer(l) {
                    ctx.err("unknown_layer", &p, format!("the design has no layer '{l}' (it has {})", f.layers.join(", ")));
                }
            }
            if kind.name == "keepout" && k == "ref" {
                if let Some(r) = v.as_str().filter(|r| !f.components.iter().any(|c| c == r)) {
                    ctx.err("unknown_component", &p, format!("the design has no component '{r}'"));
                }
            }
        }
    }
}

/// Checks `draft` (already parsed JSON). `schema` is `docs/schema/rule-draft-1.0.schema.json`.
pub fn check(draft: &Value, draft_sha256: &str, facts: Option<&DesignFacts>, schema: &Value) -> CheckReport {
    let mut ctx = Ctx { facts, errors: Vec::new(), warnings: Vec::new() };
    let mut verdicts: Vec<RuleVerdict> = Vec::new();

    // 1. structure
    for e in crate::jsonschema::validate(schema, draft) {
        let (path, msg) = e.split_once(": ").map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or(("$".into(), e.clone()));
        ctx.err("schema", &path, msg);
    }
    if ctx.errors.is_empty() {
        semantic(draft, &mut ctx, &mut verdicts);
    }

    let mut summary = Summary { rules: verdicts.len(), ..Default::default() };
    for v in &verdicts {
        match v.strength.as_str() {
            "hard" => summary.hard += 1,
            "soft" => summary.soft += 1,
            "unknown" => summary.unknown += 1,
            _ => summary.unsupported += 1,
        }
        summary.executable += usize::from(v.executable);
        summary.relaxations += usize::from(v.relaxes);
    }
    CheckReport {
        format: FORMAT,
        format_version: "1.0",
        draft_sha256: draft_sha256.to_string(),
        valid: ctx.errors.is_empty(),
        design_checked: facts.is_some(),
        errors: ctx.errors,
        warnings: ctx.warnings,
        rules: verdicts,
        summary,
    }
}

fn semantic(draft: &Value, ctx: &mut Ctx, verdicts: &mut Vec<RuleVerdict>) {
    if draft["format_version"].as_str().and_then(|s| s.split('.').next()) != Some("1") {
        ctx.err("unsupported_version", "$.format_version", format!("format_version {} is not supported (this fastroute reads 1.x)", draft["format_version"]));
        return;
    }
    scan_geometry(draft, "$", ctx);

    // 2. inputs
    let mut inputs: HashMap<String, (String, String)> = HashMap::new(); // id -> (kind, normalised text)
    for (i, inp) in draft["inputs"].as_array().into_iter().flatten().enumerate() {
        let id = inp["id"].as_str().unwrap_or("").to_string();
        let path = format!("$.inputs[{i}]");
        if id.is_empty() || inputs.contains_key(&id) {
            ctx.err("duplicate_id", &format!("{path}.id"), format!("input id '{id}' is empty or used twice"));
        }
        let kind = inp["kind"].as_str().unwrap_or("").to_string();
        let text = inp["text"].as_str().unwrap_or("");
        if kind == "natural_language" && text.trim().is_empty() {
            ctx.err("input_without_text", &path, "a natural_language input needs its text, so that the quotes can be checked");
        }
        if let (Some(f), Some(sha)) = (ctx.facts, inp["sha256"].as_str()) {
            if matches!(kind.as_str(), "ai_pack" | "inspect_json") && sha != f.inspect_sha256 {
                ctx.err("design_mismatch", &format!("{path}.sha256"), "this input is not the inspect.json that the draft is being checked against (the draft was made for another board or another state of it)");
            }
        }
        inputs.insert(id, (kind, norm(text)));
    }
    if let (Some(f), Some(sha)) = (ctx.facts, draft["design"]["input_sha256"].as_str()) {
        if f.input_sha256.as_deref().is_some_and(|d| d != sha) {
            ctx.err("design_mismatch", "$.design.input_sha256", "the draft is for another design file than the one it is checked against");
        }
    }
    if ctx.facts.is_none() {
        ctx.warn("design_unchecked", "$", "no design was given (--ai-pack / --design): nets, layers and the loosening of the design's limits could not be checked; `rules apply` requires it");
    }

    // 3. rules
    let mut ids: HashSet<String> = HashSet::new();
    for (i, rule) in draft["rules"].as_array().into_iter().flatten().enumerate() {
        let path = format!("$.rules[{i}]");
        let id = rule["id"].as_str().unwrap_or("").to_string();
        if id.trim().is_empty() || !ids.insert(id.clone()) {
            ctx.err("duplicate_id", &format!("{path}.id"), format!("rule id '{id}' is empty or used twice"));
        }
        let kind_name = rule["kind"].as_str().unwrap_or("");
        let Some(kind) = kinds::kind(kind_name) else { continue };
        let strength = rule["strength"].as_str().unwrap_or("").to_string();
        let before = ctx.errors.len();
        let relaxes = check_rule(rule, kind, &strength, &path, &inputs, ctx);
        let ok = ctx.errors.len() == before;
        let executable = ok && matches!(strength.as_str(), "hard" | "soft") && kind.capability.executable();
        let reason = if !ok {
            "the rule has errors".to_string()
        } else {
            match (strength.as_str(), kind.capability) {
                ("unknown", _) => "unknown: the value cannot be determined from the inputs, so it is never executed".into(),
                ("unsupported", cap) if !cap.executable() => format!("marked unsupported, and the engine cannot enforce it ({}): recorded, never executed", kind.maps_to),
                ("unsupported", _) => "marked unsupported: recorded, never executed".into(),
                (_, Capability::NeedsCore) => format!("the engine cannot be told this yet ({})", kind.maps_to),
                (_, Capability::Unsupported) => format!("the engine cannot enforce this ({})", kind.maps_to),
                (_, Capability::ViaKicad) => format!("executable through KiCad's rules: {}", kind.maps_to),
                (_, Capability::Supported) => format!("executable: {}", kind.maps_to),
            }
        };
        verdicts.push(RuleVerdict { id, kind: kind.name.to_string(), strength, capability: kind.capability, executable, reason, relaxes });
    }
    conflicts(draft, ctx);
}

/// Returns whether the rule (correctly) declares a loosening.
fn check_rule(rule: &Value, kind: &Kind, strength: &str, path: &str, inputs: &HashMap<String, (String, String)>, ctx: &mut Ctx) -> bool {
    // scope
    let scope = match scope_of(rule) {
        Ok(s) => Some(s),
        Err(m) => {
            ctx.err("bad_scope", &format!("{path}.applies_to"), m);
            None
        }
    };
    if let Some(scope) = &scope {
        if let Some(f) = ctx.facts {
            match scope {
                Scope::Nets(nets) => {
                    for n in nets {
                        if !f.nets.contains_key(n) {
                            let hint = f.similar_nets(n);
                            ctx.err("unknown_net", &format!("{path}.applies_to.nets"), format!("the design has no net '{n}'{}", if hint.is_empty() { String::new() } else { format!(" (similar: {})", hint.join(", ")) }));
                        }
                    }
                }
                Scope::Class(c) => {
                    if !f.classes.contains_key(c) {
                        ctx.err("unknown_net_class", &format!("{path}.applies_to.net_class"), format!("the design has no net class '{c}' (it has {})", f.classes.keys().cloned().collect::<Vec<_>>().join(", ")));
                    }
                }
                Scope::All => {}
            }
        }
        let n = if let Scope::Nets(n) = scope { n.len() } else { 0 };
        // (an unknown rule may not know its nets yet: that can be exactly what is missing)
        match kind.name {
            "diff_pair" if n != 2 && strength != "unknown" => ctx.err("bad_scope", &format!("{path}.applies_to"), "a diff_pair rule names exactly two nets"),
            "length_match" if n < 2 && strength != "unknown" => ctx.err("bad_scope", &format!("{path}.applies_to"), "a length_match rule names the nets of the group (at least two)"),
            _ => {}
        }
    }

    // strength and parameters
    let params = rule.get("params").cloned().unwrap_or(Value::Null);
    let has_params = params.as_object().is_some_and(|o| !o.is_empty());
    match strength {
        "unknown" => {
            if has_params {
                ctx.err("unknown_with_value", &format!("{path}.params"), "an unknown rule carries no value: the value is exactly what is not known (list what is missing in `needed`)");
            }
            if !rule["needed"].as_array().is_some_and(|a| !a.is_empty() && a.iter().all(|x| x.as_str().is_some_and(|s| !s.trim().is_empty()))) {
                ctx.err("unknown_without_needed", &format!("{path}.needed"), "an unknown rule says what is needed to make it a real rule");
            }
            if rule.get("relaxes").is_some() {
                ctx.err("relaxes_unknown", &format!("{path}.relaxes"), "an unknown rule has no value, so it cannot loosen anything");
            }
        }
        "hard" | "soft" | "unsupported" => {
            if !params.is_null() && !params.is_object() {
                ctx.err("bad_param", &format!("{path}.params"), "params must be an object");
            }
            check_param_types(kind, &params, &format!("{path}.params"), ctx);
            let present = |k: &str| params.get(k).is_some();
            let complete = kind.required.iter().all(|k| present(k)) && (kind.one_of.is_empty() || kind.one_of.iter().any(|k| present(k)));
            if (strength != "unsupported" || has_params) && !complete {
                ctx.err("missing_param", &format!("{path}.params"), format!("a {} rule needs {}", kind.name, if kind.required.is_empty() { format!("one of {}", kind.one_of.join(", ")) } else { kind.required.join(", ") }));
            }
            if strength != "unsupported" && !kind.capability.executable() {
                ctx.err(
                    "cannot_enforce",
                    &format!("{path}.strength"),
                    format!("a {} rule cannot be enforced by this engine ({}); mark it \"unsupported\" so that nobody believes it is in effect", kind.name, kind.maps_to),
                );
            }
        }
        _ => {}
    }
    if strength != "soft" && rule.get("weight").is_some() {
        ctx.warn("weight_ignored", &format!("{path}.weight"), "weight only means something for soft rules");
    }

    // evidence
    let mut explicit_quotes: Vec<String> = Vec::new();
    for (j, ev) in rule["evidence"].as_array().into_iter().flatten().enumerate() {
        let ep = format!("{path}.evidence[{j}]");
        let input_id = ev["input"].as_str().unwrap_or("");
        let Some((input_kind, text)) = inputs.get(input_id) else {
            ctx.err("evidence_input_missing", &format!("{ep}.input"), format!("there is no input '{input_id}'"));
            continue;
        };
        let ev_kind = ev["kind"].as_str().unwrap_or("");
        match ev_kind {
            "natural_language" | "human" => {
                if input_kind != "natural_language" {
                    ctx.err("evidence_input_kind", &ep, format!("'{ev_kind}' evidence must point at a natural_language input, '{input_id}' is {input_kind}"));
                    continue;
                }
                let quote = ev["quote"].as_str().unwrap_or("");
                if quote.trim().is_empty() {
                    ctx.err("evidence_quote_missing", &ep, "name the words of the input that ask for this rule (quote)");
                } else if !text.contains(&norm(quote)) {
                    ctx.err("quote_not_found", &format!("{ep}.quote"), format!("the quote \"{quote}\" does not occur in input '{input_id}': a rule cannot be traced to words that are not there"));
                } else {
                    explicit_quotes.push(quote.to_string());
                }
            }
            "design_data" | "reference_board" => {
                let want = if ev_kind == "design_data" { ["ai_pack", "inspect_json"].as_slice() } else { ["reference_board"].as_slice() };
                if !want.contains(&input_kind.as_str()) {
                    ctx.err("evidence_input_kind", &ep, format!("'{ev_kind}' evidence must point at an input of kind {}, '{input_id}' is {input_kind}", want.join(" or ")));
                }
                if ev["ref"].as_str().is_none_or(|r| r.trim().is_empty()) {
                    ctx.err("evidence_ref_missing", &ep, "say what was read (ref), e.g. nets[USB_DP].class");
                }
                if let (Some(f), "design_data", Some(r)) = (ctx.facts, ev_kind, ev["ref"].as_str()) {
                    if let Some(name) = r.strip_prefix("nets[").and_then(|x| x.split(']').next()) {
                        if !f.nets.contains_key(name) {
                            ctx.err("evidence_ref_not_found", &format!("{ep}.ref"), format!("the design has no net '{name}'"));
                        }
                    }
                    if let Some(name) = r.strip_prefix("layers[").and_then(|x| x.split(']').next()) {
                        if !f.has_layer(name) {
                            ctx.err("evidence_ref_not_found", &format!("{ep}.ref"), format!("the design has no layer '{name}'"));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // values that cannot be read off a board: a person has to have said them
    if kind.explicit_source && strength != "unknown" && has_params {
        for (key, value) in params.as_object().into_iter().flatten().filter_map(|(k, v)| v.as_f64().map(|x| (k, x))) {
            if explicit_quotes.is_empty() {
                ctx.err(
                    "value_without_source",
                    &format!("{path}.params.{key}"),
                    format!("{} cannot be read from a board or a picture; without a person's words that state {value} the rule must be \"unknown\" (with `needed`), not a number that was guessed", kind.name),
                );
            } else if !explicit_quotes.iter().any(|q| numbers_in(q).iter().any(|n| (n - value).abs() < 1e-9)) {
                ctx.err("value_not_in_quote", &format!("{path}.params.{key}"), format!("the number {value} does not occur in the quoted words; a {} value must be stated, not derived", kind.name));
            }
        }
    }

    // loosening what the design demands
    let mut declared = false;
    if let (Some(scope), true) = (&scope, matches!(strength, "hard" | "soft")) {
        let actual = ctx.facts.and_then(|f| baseline(kind.name, &params, scope, f));
        let relax = rule.get("relaxes");
        match (actual, relax) {
            (Some((param, base, true)), None) => ctx.err(
                "silent_relaxation",
                &format!("{path}.params.{param}"),
                format!("this loosens {param} of the design from {base} to {} ; a draft may not do that silently: declare it with `relaxes` {{parameter, baseline, reason}}, and a person has to accept it when confirming", params[param]),
            ),
            (Some((param, base, true)), Some(r)) => {
                declared = true;
                if r["parameter"].as_str() != Some(param) {
                    ctx.err("relaxation_mismatch", &format!("{path}.relaxes.parameter"), format!("the loosened parameter is {param}"));
                }
                if r["baseline"].as_f64().is_none_or(|b| (b - base).abs() > 1e-6) {
                    ctx.err("relaxation_mismatch", &format!("{path}.relaxes.baseline"), format!("the design's limit is {base}, not {}: a declaration must state the real baseline", r["baseline"]));
                }
                if r["reason"].as_str().is_none_or(|s| s.trim().is_empty()) {
                    ctx.err("relaxation_mismatch", &format!("{path}.relaxes.reason"), "say why");
                }
            }
            (Some((_, _, false)), Some(_)) | (None, Some(_)) if ctx.facts.is_some() => {
                ctx.warn("relaxation_not_needed", &format!("{path}.relaxes"), "this rule does not loosen anything of the design; the declaration is not needed");
            }
            (None, None) if ctx.facts.is_none() && matches!(kind.name, "clearance" | "trace_width" | "length_max") && strength == "hard" => {
                ctx.warn("baseline_unknown", path, "without the design it is not known whether this rule loosens a limit of the design");
            }
            _ => {}
        }
    } else if rule.get("relaxes").is_some() && !matches!(strength, "unknown") {
        ctx.warn("relaxation_not_needed", &format!("{path}.relaxes"), "only hard and soft rules can loosen a limit");
    }
    declared
}

fn conflicts(draft: &Value, ctx: &mut Ctx) {
    // two executable rules of one kind on the same net with different values
    let mut seen: BTreeMap<(String, String, String), (String, f64)> = BTreeMap::new();
    for rule in draft["rules"].as_array().into_iter().flatten() {
        let (Some(kind), Some(id)) = (rule["kind"].as_str(), rule["id"].as_str()) else { continue };
        if !matches!(rule["strength"].as_str(), Some("hard") | Some("soft")) {
            continue;
        }
        let Ok(scope) = scope_of(rule) else { continue };
        let nets: Vec<String> = match &scope {
            Scope::Nets(n) => n.clone(),
            Scope::Class(c) => vec![format!("class:{c}")],
            Scope::All => vec!["*".into()],
        };
        for (key, v) in rule["params"].as_object().into_iter().flatten().filter_map(|(k, v)| v.as_f64().map(|x| (k.clone(), x))) {
            for n in &nets {
                let k = (kind.to_string(), n.clone(), key.clone());
                match seen.get(&k) {
                    Some((other, ov)) if (ov - v).abs() > 1e-9 => ctx.warn("conflicting_rules", "$.rules", format!("rules {other} and {id} both set {key} of {kind} on {n} ({ov} and {v})")),
                    Some(_) => {}
                    None => {
                        seen.insert(k, (id.to_string(), v));
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_found_in_words() {
        assert_eq!(numbers_in("USB 90 ohm, gap 0.15 mm"), vec![90.0, 0.15]);
        assert_eq!(numbers_in("3A at .5mm"), vec![3.0, 0.5]);
        assert_eq!(numbers_in("rev 1.2.3"), vec![1.2, 3.0]);
        assert_eq!(numbers_in("no numbers."), Vec::<f64>::new());
        assert_eq!(numbers_in("ends with 7."), vec![7.0]);
    }

    #[test]
    fn baselines_and_loosening() {
        use super::super::facts::ClassFacts;
        let mut f = DesignFacts::default();
        f.nets.insert("A".into(), "default".into());
        f.classes.insert(
            "default".into(),
            ClassFacts { trace_width_mm: vec![("F.Cu".into(), 0.25)], clearance_mm: vec![("F.Cu".into(), 0.2), ("B.Cu".into(), 0.3)], max_length_mm: Some(80.0) },
        );
        let s = Scope::Nets(vec!["A".into()]);
        let p = |k: &str, v: f64| serde_json::json!({ k: v });
        assert_eq!(baseline("clearance", &p("clearance_mm", 0.1), &s, &f), Some(("clearance_mm", 0.3, true)));
        assert_eq!(baseline("clearance", &p("clearance_mm", 0.3), &s, &f), Some(("clearance_mm", 0.3, false)));
        assert_eq!(baseline("clearance", &serde_json::json!({"clearance_mm": 0.25, "layer": "F.Cu"}), &s, &f), Some(("clearance_mm", 0.2, false)));
        assert_eq!(baseline("trace_width", &p("width_mm", 0.2), &s, &f), Some(("width_mm", 0.25, true)));
        assert_eq!(baseline("trace_width", &p("width_mm", 0.5), &s, &f), Some(("width_mm", 0.25, false)));
        assert_eq!(baseline("length_max", &p("max_mm", 100.0), &s, &f), Some(("max_mm", 80.0, true)));
        assert_eq!(baseline("length_max", &p("max_mm", 50.0), &s, &f), Some(("max_mm", 80.0, false)));
        assert_eq!(baseline("diff_pair", &p("gap_mm", 0.1), &s, &f), None);
        assert_eq!(baseline("clearance", &p("clearance_mm", 0.1), &Scope::Nets(vec!["nope".into()]), &f), None);
    }
}
