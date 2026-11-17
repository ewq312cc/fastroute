//! `fastroute rules draft`: a minimal, offline way to turn written requirements (and the data of
//! an ai-pack) into a rule draft. It is a transparent keyword grammar, not an AI: it needs no
//! network and no vendor, it quotes the sentence every rule comes from, and it never guesses -
//! what it cannot read becomes an `unknown` rule that says what is missing, and what the engine
//! cannot enforce becomes `unsupported`. An AI model can write the same draft format; the
//! checker (`rules check`) treats both alike.
//!
//! Understood (English, a few Chinese words): differential pairs with gap / skew / impedance,
//! trace width ("widen VBUS to 0.5 mm"), clearance, maximum length, layer preference or
//! restriction, current, timing. Strength: must / shall / never / required / 必须 / 不得 -> hard;
//! prefer / try / should / ideally / 尽量 / 优先 -> soft; a plain numeric limit is hard.

use serde_json::{json, Value};

use super::check::{baseline, numbers_in};
use super::facts::{DesignFacts, Scope};
use crate::load::sha256_hex;

pub struct Generated {
    pub draft: Value,
    pub notes: Vec<String>,
}

// ("at least", "no more than" describe a limit, not how binding the requirement is)
const HARD: &[&str] = &["must", "shall", "never", "required", "require ", "必须", "不得", "严禁", "禁止"];
const SOFT: &[&str] = &["prefer", "try to", "try ", "ideally", "if possible", "should", "尽量", "优先", "最好"];

fn sentences(text: &str) -> Vec<&str> {
    let b: Vec<(usize, char)> = text.char_indices().collect();
    let mut out = Vec::new();
    let mut start = 0;
    for (k, &(i, c)) in b.iter().enumerate() {
        let next = b.get(k + 1).map(|x| x.1);
        let prev = if k > 0 { Some(b[k - 1].1) } else { None };
        let end = match c {
            '\n' | ';' | '。' | '；' | '!' | '?' => true,
            // not the decimal point of 0.15, nor the dot of "F.Cu" or "U1.2"
            '.' => next.is_none_or(char::is_whitespace) && !prev.is_some_and(|p| p.is_ascii_digit()),
            _ => false,
        };
        if end {
            out.push(&text[start..i]);
            start = i + c.len_utf8();
        }
    }
    out.push(&text[start..]);
    out.into_iter().map(str::trim).filter(|s| !s.is_empty()).collect()
}

fn has(l: &str, words: &[&str]) -> bool {
    words.iter().any(|w| l.contains(w))
}

fn strength_of(l: &str, default: &'static str) -> &'static str {
    if has(l, HARD) {
        "hard"
    } else if has(l, SOFT) {
        "soft"
    } else {
        default
    }
}

/// The first number after one of `kws` within 40 characters, in mm (`mil`, `um`, `cm` converted).
fn mm_after(l: &str, kws: &[&str]) -> Option<f64> {
    for kw in kws {
        let Some(i) = l.find(kw) else { continue };
        let window: String = l[i + kw.len()..].chars().take(40).collect();
        let Some(v) = numbers_in(&window).first().copied() else { continue };
        // the unit right after that number
        let digits_end = window.find(|c: char| c.is_ascii_digit()).map(|s| {
            let rest = &window[s..];
            s + rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len())
        });
        let tail: String = digits_end.map(|e| window[e..].trim_start().chars().take(4).collect()).unwrap_or_default();
        let mm = if tail.starts_with("mil") {
            v * 0.0254
        } else if tail.starts_with("um") || tail.starts_with("µm") {
            v / 1000.0
        } else if tail.starts_with("cm") {
            v * 10.0
        } else {
            v
        };
        return Some((mm * 1e6).round() / 1e6);
    }
    None
}

/// The number (with its unit) written just before one of `kws` within 16 characters:
/// `0.4 mm wide`.
fn mm_before(l: &str, kws: &[&str]) -> Option<f64> {
    for kw in kws {
        let Some(i) = l.find(kw) else { continue };
        let head: Vec<char> = l[..i].chars().rev().take(16).collect::<Vec<_>>().into_iter().rev().collect();
        let head: String = head.into_iter().collect();
        let Some(v) = numbers_in(&head).last().copied() else { continue };
        let unit = head.rsplit(|c: char| c.is_ascii_digit() || c == '.').next().unwrap_or("").trim();
        let mm = if unit.starts_with("mil") {
            v * 0.0254
        } else if unit.starts_with("um") || unit.starts_with("µm") {
            v / 1000.0
        } else if unit.starts_with("cm") {
            v * 10.0
        } else {
            v
        };
        return Some((mm * 1e6).round() / 1e6);
    }
    None
}

/// A number directly in front of one of `units` (`90 ohm`, `3 a`).
fn number_before(l: &str, units: &[&str]) -> Option<f64> {
    for u in units {
        let mut from = 0;
        while let Some(i) = l[from..].find(u) {
            let at = from + i;
            let head: String = l[..at].chars().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
            let trimmed = head.trim_end();
            // the unit must stand alone ("3 a", not the "a" in "a pair")
            let after = l[at + u.len()..].chars().next();
            if after.is_none_or(|c| !c.is_alphanumeric()) {
                if let Some(v) = numbers_in(trimmed).last().copied() {
                    if trimmed.chars().last().is_some_and(|c| c.is_ascii_digit() || c == '.') {
                        return Some(v);
                    }
                }
            }
            from = at + u.len();
        }
    }
    None
}

fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '+' | '-' | '/' | '.')))
        .map(|t| t.trim_end_matches(['.', ',']).to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Nets of the design that the sentence names (whole words, any case, with or without a leading `/`).
fn nets_in(sentence: &str, facts: Option<&DesignFacts>) -> Vec<String> {
    let toks: Vec<String> = tokens(sentence).into_iter().map(|t| t.to_lowercase()).collect();
    let Some(f) = facts else {
        // without the design only "net NAME" can be trusted
        let raw = tokens(sentence);
        return raw.windows(2).filter(|w| matches!(w[0].to_lowercase().as_str(), "net" | "nets")).map(|w| w[1].clone()).collect();
    };
    // (a net named "A" would match the article: names of one character are not matched in prose)
    f.nets.keys().filter(|n| n.trim_start_matches('/').chars().count() >= 2).filter(|n| {
        let l = n.to_lowercase();
        toks.iter().any(|t| *t == l || *t == l.trim_start_matches('/'))
    }).cloned().collect()
}

/// `(base, is_positive)` of a differential net name.
fn pair_part(name: &str) -> Option<(String, bool)> {
    let u = name.to_uppercase();
    for (p, n) in [("_DP", "_DN"), ("_DP", "_DM"), ("_P", "_N"), ("+", "-"), ("P", "N")] {
        for (suffix, pos) in [(p, true), (n, false)] {
            if let Some(base) = u.strip_suffix(suffix) {
                // "P"/"N" alone only after a separator or digit, or "VCCP" would be a pair half
                if suffix.len() == 1 && suffix.chars().all(|c| c.is_alphabetic()) && !base.ends_with(|c: char| c == '_' || c.is_ascii_digit()) {
                    continue;
                }
                if !base.is_empty() {
                    return Some((base.to_string(), pos));
                }
            }
        }
    }
    None
}

/// Pairs of the design that the sentence is about: both nets named, or the pair's base name
/// (`/USB_D`) or its first word (`usb`) named.
fn pairs_in(sentence: &str, facts: Option<&DesignFacts>) -> Vec<(String, String)> {
    let l = sentence.to_lowercase();
    let toks = tokens(&l);
    let Some(f) = facts else {
        // "A/B" written out
        return tokens(sentence).into_iter().filter_map(|t| t.split_once('/').filter(|(a, b)| !a.is_empty() && !b.is_empty()).map(|(a, b)| (a.to_string(), b.to_string()))).collect();
    };
    let mut halves: std::collections::BTreeMap<String, (Option<String>, Option<String>)> = Default::default();
    for n in f.nets.keys() {
        if let Some((base, pos)) = pair_part(n) {
            let e = halves.entry(base).or_default();
            if pos { e.0 = Some(n.clone()) } else { e.1 = Some(n.clone()) }
        }
    }
    let mut out = Vec::new();
    for (base, (p, n)) in halves {
        let (Some(p), Some(n)) = (p, n) else { continue };
        let b = base.trim_start_matches('/').to_lowercase();
        let first = b.split(['_', '-']).next().unwrap_or("").to_string();
        let named = toks.iter().any(|t| *t == b || t.trim_start_matches('/') == b || (first.len() >= 3 && *t == first));
        let both = nets_in(sentence, Some(f)).iter().filter(|x| **x == p || **x == n).count() == 2;
        if named || both {
            out.push((p, n));
        }
    }
    out
}

struct Builder<'a> {
    rules: Vec<Value>,
    notes: Vec<String>,
    facts: Option<&'a DesignFacts>,
}

impl Builder<'_> {
    #[allow(clippy::too_many_arguments)] // (a rule is exactly these things; a struct would only rename them)
    fn add(&mut self, sentence: &str, kind: &str, strength: &str, scope: &Scope, params: Value, needed: &[&str], rationale: &str) {
        let id = format!("R{}", self.rules.len() + 1);
        let applies_to = match scope {
            Scope::Nets(n) => json!({"nets": n}),
            Scope::Class(c) => json!({"net_class": c}),
            Scope::All => json!({"all": true}),
        };
        let mut evidence = vec![json!({"input": "nl1", "kind": "natural_language", "quote": sentence})];
        if let (Some(_), Scope::Nets(n)) = (self.facts, scope) {
            evidence.push(json!({"input": "pack", "kind": "design_data", "ref": format!("nets[{}].class", n[0])}));
        }
        let mut rule = json!({"id": id, "kind": kind, "strength": strength, "applies_to": applies_to, "evidence": evidence, "rationale": rationale});
        if strength == "unknown" {
            rule["needed"] = json!(needed);
        } else {
            rule["params"] = params.clone();
            // a rule that loosens what the design demands says so, with the design's real limit
            if let (Some(f), true) = (self.facts, matches!(strength, "hard" | "soft")) {
                if let Some((param, base, true)) = baseline(kind, &params, scope, f) {
                    rule["relaxes"] = json!({"parameter": param, "baseline": base, "reason": format!("the requirement asks for {} while the design requires {base}", params[param])});
                    self.notes.push(format!("{id}: loosens {param} of the design (from {base}); it is declared, and a person must accept it when confirming"));
                }
            }
        }
        self.rules.push(rule);
    }

    fn unknown(&mut self, sentence: &str, kind: &str, scope: &Scope, needed: &[&str], why: &str) {
        self.add(sentence, kind, "unknown", scope, Value::Null, needed, why);
    }
}

/// Reads `text`; `pack` is `(kind, path)` of the inspect.json / ai-pack the facts came from.
pub fn generate(text: &str, facts: Option<&DesignFacts>, pack: Option<(&str, &str)>) -> Generated {
    let mut b = Builder { rules: Vec::new(), notes: Vec::new(), facts };
    for s in sentences(text) {
        let l = s.to_lowercase();
        let nets = nets_in(s, facts);
        let scope_nets = if nets.is_empty() { Scope::All } else { Scope::Nets(nets.clone()) };
        let mut understood = false;

        // differential pairs, with impedance
        if has(&l, &["differential", "diff pair", "diff-pair", "差分"]) {
            understood = true;
            let pairs = pairs_in(s, facts);
            let st = strength_of(&l, "hard");
            let gap = mm_after(&l, &["gap"]);
            let skew = mm_after(&l, &["skew"]);
            // "the USB pair should have 90 ohm" is about impedance, not about the gap
            let impedance_only = gap.is_none() && skew.is_none() && has(&l, &["ohm", "impedance", "Ω", "阻抗", "欧"]);
            if pairs.is_empty() && !impedance_only {
                b.unknown(s, "diff_pair", &Scope::All, &["the two nets of the differential pair (their exact names)"], "the sentence asks for a differential pair but no pair of the design could be identified");
            }
            for (p, n) in &pairs {
                b.notes.push(format!("\"{s}\": read as the pair {p} / {n} of the design"));
            }
            for (p, n) in pairs.iter().filter(|_| !impedance_only) {
                let scope = Scope::Nets(vec![p.clone(), n.clone()]);
                let mut params = serde_json::Map::new();
                if let Some(g) = gap {
                    params.insert("gap_mm".into(), json!(g));
                }
                if let Some(k) = skew {
                    params.insert("skew_mm".into(), json!(k));
                }
                if params.is_empty() {
                    b.unknown(s, "diff_pair", &scope, &["the gap between the two traces (mm)", "the allowed length difference (mm)"], "a pair was found, but no gap or skew value is written");
                } else {
                    b.add(s, "diff_pair", st, &scope, Value::Object(params), &[], "pairs are routed first and coupled at the gap, then length-matched");
                }
            }
            let pair_scope = if pairs.is_empty() { scope_nets.clone() } else { Scope::Nets(pairs.iter().flat_map(|(p, n)| [p.clone(), n.clone()]).collect()) };
            if let Some(ohm) = number_before(&l, &["ohm", "ohms", "Ω", "欧"]) {
                b.add(s, "impedance", st, &pair_scope, json!({"target_ohm": ohm, "differential": true}), &[], "the number is the one written in the requirement; the widths follow from the board's stackup (impedance_cli.py)");
            }
        }
        if has(&l, &["impedance", "阻抗"]) && !has(&l, &["ohm", "Ω", "欧"]) {
            understood = true;
            b.unknown(s, "impedance", &scope_nets, &["the target impedance in ohm", "the board stackup (layer heights, dielectric constant)"], "impedance is asked for without a value; it cannot be read off a board or a picture");
        } else if has(&l, &["ohm", "Ω", "欧"]) && !has(&l, &["differential", "diff pair", "diff-pair", "差分"]) {
            understood = true;
            if let Some(ohm) = number_before(&l, &["ohm", "ohms", "Ω", "欧"]) {
                b.add(s, "impedance", strength_of(&l, "hard"), &scope_nets, json!({"target_ohm": ohm, "differential": false}), &[], "the number is the one written in the requirement");
            }
        }

        // trace width
        if has(&l, &["width", "widen", "wide", "线宽", "加粗"]) && !has(&l, &["impedance", "阻抗"]) {
            understood = true;
            let mut scope = scope_nets.clone();
            let mut why = "width as written in the requirement".to_string();
            if matches!(scope, Scope::All) {
                if let (Some(f), true) = (facts, has(&l, &["power", "电源"])) {
                    let power: Vec<String> = f.nets.keys().filter(|n| {
                        let u = n.to_lowercase();
                        ["vcc", "vdd", "vbus", "vin", "vbat", "+5v", "+3v3", "5v", "3v3", "+12v"].iter().any(|p| u.contains(p))
                    }).cloned().collect();
                    if !power.is_empty() {
                        b.notes.push(format!("\"power\" was read as the nets {} (names that look like supplies); check the list", power.join(", ")));
                        why = format!("\"power\" was read as: {}", power.join(", "));
                        scope = Scope::Nets(power);
                    }
                }
            }
            let width = mm_after(&l, &["width", "widen to", "widen", "线宽", "加粗"]).or_else(|| mm_before(&l, &["wide"]));
            match (width, &scope) {
                (Some(w), Scope::Nets(_)) | (Some(w), Scope::Class(_)) => b.add(s, "trace_width", strength_of(&l, "hard"), &scope, json!({"width_mm": w}), &[], &why),
                (Some(_), Scope::All) => b.unknown(s, "trace_width", &Scope::All, &["which nets or net class the width is for"], "a width is written but not for which nets"),
                (None, _) => b.unknown(s, "trace_width", &scope, &["the width in mm"], "a wider trace is asked for without a value"),
            }
        }

        // clearance (a pair's "gap" is handled above)
        if has(&l, &["clearance", "spacing", "间距"]) && !(has(&l, &["differential", "差分"]) && l.contains("gap")) {
            understood = true;
            match mm_after(&l, &["clearance", "spacing", "间距"]) {
                Some(c) => b.add(s, "clearance", strength_of(&l, "hard"), &scope_nets, json!({"clearance_mm": c}), &[], "clearance as written in the requirement"),
                None => b.unknown(s, "clearance", &scope_nets, &["the clearance in mm"], "clearance is mentioned without a value"),
            }
        }

        // maximum length
        if has(&l, &["max length", "maximum length", "no longer than", "not longer than", "not exceed", "最大线长", "不超过"]) {
            understood = true;
            match (mm_after(&l, &["max length", "maximum length", "longer than", "not exceed", "最大线长", "不超过"]), &scope_nets) {
                (Some(m), Scope::Nets(_)) => b.add(s, "length_max", strength_of(&l, "hard"), &scope_nets, json!({"max_mm": m}), &[], "length limit as written; the router does not enforce it, KiCad lists nets that exceed it"),
                (Some(_), _) => b.unknown(s, "length_max", &Scope::All, &["which nets the length limit is for"], "a length is written but not for which nets"),
                (None, _) => b.unknown(s, "length_max", &scope_nets, &["the length in mm"], "a length limit is mentioned without a value"),
            }
        }

        // layers
        let layer_names: Vec<String> = facts
            .map(|f| {
                let mut v: Vec<String> = f.layers.iter().filter(|n| tokens(&l).iter().any(|t| *t == n.to_lowercase())).cloned().collect();
                if has(&l, &["top layer", " top ", "top side"]) {
                    v.extend(f.layers.first().cloned());
                }
                if has(&l, &["bottom layer", " bottom ", "bottom side"]) {
                    v.extend(f.layers.last().cloned());
                }
                v.sort();
                v.dedup();
                v
            })
            .unwrap_or_default();
        let restrict = has(&l, &["only on", "restricted to", "restrict ", "仅"]);
        if restrict || has(&l, &["prefer", "尽量", "优先", "route on", "routed on"]) {
            if layer_names.is_empty() {
                if has(&l, &["layer", "top", "bottom", "f.cu", "b.cu", "层"]) {
                    understood = true;
                    b.unknown(s, if restrict { "layer_restriction" } else { "layer_preference" }, &scope_nets, &["the exact layer names of the design (give the design with --ai-pack / --design)"], "layers are mentioned but could not be matched to layers of the design");
                }
            } else {
                understood = true;
                let (kind, key) = if restrict { ("layer_restriction", "allowed_layers") } else { ("layer_preference", "preferred_layers") };
                // neither can be enforced by this engine: recorded as unsupported, never as a rule in effect
                b.add(s, kind, "unsupported", &scope_nets, json!({ key: layer_names }), &[], "the engine cannot enforce this (see the capability table); kept as a record of what was asked");
            }
        }

        // current and timing: only what a person stated
        if has(&l, &["carry", "carries", "amp", "载流", "current capacity", "current of", "rated current"]) {
            understood = true;
            match number_before(&l, &["a", "amp", "amps", "ampere", "amperes", "安"]) {
                Some(a) => b.add(s, "current_capacity", "unsupported", &scope_nets, json!({"current_a": a}), &[], "the current is the one written in the requirement; the engine has no current model (give the width as a trace_width rule)"),
                None => b.unknown(s, "current_capacity", &scope_nets, &["the current in A", "the allowed temperature rise", "copper thickness"], "current is mentioned without a value; it cannot be read off a board"),
            }
        }
        if has(&l, &["timing", "delay", "latency", "时序", "high-speed", "high speed", "高速"]) {
            understood = true;
            match (number_before(&l, &["ps"]), number_before(&l, &["ns"])) {
                (Some(ps), _) => b.add(s, "timing_delay", "unsupported", &scope_nets, json!({"max_delay_ps": ps}), &[], "the value is the one written; the router matches lengths, not delays"),
                (None, Some(ns)) => b.add(s, "timing_delay", "unsupported", &scope_nets, json!({"max_delay_ps": ns * 1000.0}), &[], "the value is the one written (ns converted to ps); the router matches lengths, not delays"),
                (None, None) => b.unknown(s, "timing_delay", &scope_nets, &["the maximum delay or skew in ps / ns", "the propagation model (stackup)"], "timing is mentioned without a value"),
            }
        }
        if !understood {
            b.notes.push(format!("not understood, no rule made: \"{s}\""));
        }
    }
    if b.rules.is_empty() {
        b.notes.push("no rule could be made from the text".into());
    }
    b.notes.push("made by the keyword prototype of `fastroute rules draft`; it quotes the sentence of every rule and never guesses a value. Check it with `fastroute rules check` like any other draft".into());

    let mut inputs = vec![json!({"id": "nl1", "kind": "natural_language", "text": text, "sha256": sha256_hex(text.as_bytes())})];
    if let (Some(f), Some((kind, path))) = (facts, pack) {
        inputs.push(json!({"id": "pack", "kind": kind, "path": path, "sha256": f.inspect_sha256}));
    }
    let design = facts.map(|f| json!({"input_sha256": f.input_sha256, "inspect_sha256": f.inspect_sha256}));
    let draft = json!({
        "format": "fastroute-rule-draft",
        "format_version": "1.0",
        "created_by": {"kind": "script", "tool": "fastroute rules draft (keyword prototype)", "model": null},
        "design": design,
        "inputs": inputs,
        "rules": b.rules,
        "notes": b.notes,
    });
    Generated { notes: draft["notes"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect()).unwrap_or_default(), draft }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentences_keep_decimals_and_layer_names() {
        assert_eq!(sentences("Gap 0.15 mm. Route on F.Cu; keep 3.3 V clean\nUSB 90 ohm."), vec!["Gap 0.15 mm", "Route on F.Cu", "keep 3.3 V clean", "USB 90 ohm"]);
        assert_eq!(sentences("one。two；three"), vec!["one", "two", "three"]);
        assert!(sentences("   ").is_empty());
    }

    #[test]
    fn values_and_units() {
        assert_eq!(mm_after("clearance of 0.2 mm", &["clearance"]), Some(0.2));
        assert_eq!(mm_after("width 10 mil", &["width"]), Some(0.254));
        assert_eq!(mm_after("width 150 um", &["width"]), Some(0.15));
        assert_eq!(mm_after("width 1.5cm", &["width"]), Some(15.0));
        assert_eq!(mm_after("width unknown", &["width"]), None);
        assert_eq!(number_before("usb 90 ohm pair", &["ohm"]), Some(90.0));
        assert_eq!(number_before("carries 3 a", &["a"]), Some(3.0));
        assert_eq!(number_before("a pair of nets", &["a"]), None, "the article 'a' is not an ampere");
    }

    #[test]
    fn pair_halves() {
        assert_eq!(pair_part("USB_DP"), Some(("USB".into(), true)));
        assert_eq!(pair_part("/USB_D+"), Some(("/USB_D".into(), true)));
        assert_eq!(pair_part("ETH_TX_N"), Some(("ETH_TX".into(), false)));
        assert_eq!(pair_part("CLK1P"), Some(("CLK1".into(), true)));
        assert_eq!(pair_part("VCCP"), None, "a name that merely ends in P is not a pair half");
        assert_eq!(pair_part("GND"), None);
    }
}
