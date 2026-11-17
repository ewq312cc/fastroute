//! Inheritance between packs (`hcp_version` 1.1): a module pack may take named things from a chip pack, a board pack from a module
//! or chip pack. Nothing is inherited that is not listed (`pack.upstream[].inherit`), an upstream is an exact id and version pinned
//! by the canonical digest of its HCP document, and a rule that replaces an inherited one (`overrides`) needs evidence from a source of
//! the derived pack itself. The result is one flat pack that `check`, `compile` and `validate` read like any other. No network, no
//! clock, no AI: the same inputs give the same bytes.

use crate::diag::Diagnostic;
use crate::hcp::{self, canonical_json, sha256_hex};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// The digest a derived pack pins its upstream by: sha-256 of the canonical JSON of the HCP document (so reformatting a file
/// does not change it, a changed fact does).
pub fn canonical_sha256(pack: &Value) -> String {
    sha256_hex(canonical_json(pack).as_bytes())
}

/// The kinds of pack a pack of this kind may inherit from. A chip pack inherits from nothing; a module from a chip; a board from a
/// module or a chip.
pub fn allowed_upstream_kinds(kind: &str) -> &'static [&'static str] {
    match kind {
        "module" => &["chip"],
        "board" => &["module", "chip"],
        _ => &[],
    }
}

fn s<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn ids(v: &Value, key: &str) -> BTreeSet<String> {
    arr(v, key).iter().filter_map(Value::as_str).map(str::to_string).collect()
}

/// Every `source_id` that evidence anywhere inside `v` cites.
fn cited_sources(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(ev)) = m.get("evidence") {
                for e in ev {
                    if let Some(id) = s(e, "source_id") {
                        out.insert(id.to_string());
                    }
                }
            }
            for c in m.values() {
                cited_sources(c, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|c| cited_sources(c, out)),
        _ => {}
    }
}

fn by_id<'a>(pack: &'a Value, section: &str, key: &str) -> BTreeMap<&'a str, &'a Value> {
    arr(pack, section).iter().filter_map(|o| s(o, key).map(|id| (id, o))).collect()
}

/// Merges what `derived` inherits from `upstreams` (the HCP documents it names; others in the list are ignored) into one pack.
pub fn resolve(derived: &Value, upstreams: &[Value]) -> Result<Value, Vec<Diagnostic>> {
    let mut errs: Vec<Diagnostic> = Vec::new();
    let shape: Vec<Diagnostic> = hcp::check_with(derived, true).into_iter().filter(|d| d.severity == crate::diag::Severity::Error).collect();
    if !shape.is_empty() {
        return Err(shape);
    }
    let pack_meta = derived.get("pack").cloned().unwrap_or(Value::Null);
    let kind = s(&pack_meta, "kind").unwrap_or("").to_string();
    let wanted = arr(&pack_meta, "upstream");
    if wanted.is_empty() {
        // nothing to merge: it is already flat
        return Ok(derived.clone());
    }
    if kind.is_empty() {
        errs.push(Diagnostic::error("kind_required", "$.pack.kind", None, "a pack that inherits says what it is (pack.kind: module or board)"));
    }
    let allowed = allowed_upstream_kinds(&kind);
    if !kind.is_empty() && allowed.is_empty() {
        errs.push(Diagnostic::error("upstream_kind_not_allowed", "$.pack.upstream", None, format!("a {kind} pack inherits from nothing; only a module pack (from a chip) or a board pack (from a module or chip) does")));
    }

    let own_sources: BTreeSet<String> = arr(derived, "sources").iter().filter_map(|x| s(x, "source_id")).map(str::to_string).collect();

    let mut devices: Vec<Value> = Vec::new();
    let mut blocks: Vec<Value> = Vec::new();
    let mut regions: Vec<Value> = Vec::new();
    let mut rules: Vec<Value> = Vec::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut inherited_rule_ids: BTreeMap<String, String> = BTreeMap::new();
    let mut provenance: Map<String, Value> = Map::new();
    let mut resolved_from: Vec<Value> = Vec::new();
    // upstream source id -> the source of this pack that is the same document; devices it applies to besides its own
    let mut alias: BTreeMap<String, String> = BTreeMap::new();
    let mut extra_applies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    for (i, u) in wanted.iter().enumerate() {
        let path = format!("$.pack.upstream[{i}]");
        let (uid, uver, ukind) = (s(u, "id").unwrap_or(""), s(u, "version").unwrap_or(""), s(u, "kind").unwrap_or(""));
        let label = format!("{uid}@{uver}");
        let Some(up) = upstreams.iter().find(|p| s(&p["pack"], "id") == Some(uid) && s(&p["pack"], "version") == Some(uver)) else {
            errs.push(Diagnostic::error("upstream_missing", path, None, format!("the upstream pack {label} was not supplied (an exact id and version is needed; a similar pack is never used instead)")));
            continue;
        };
        let digest = canonical_sha256(up);
        if Some(digest.as_str()) != s(u, "canonical_sha256") {
            errs.push(Diagnostic::error("upstream_digest_mismatch", path.clone(), None, format!("{label} has the canonical digest {digest}, but the pack pins {}: the upstream changed after the pack was written, so it has to be reviewed again", s(u, "canonical_sha256").unwrap_or(""))));
            continue;
        }
        match s(&up["pack"], "kind") {
            None => {
                errs.push(Diagnostic::error("upstream_kind_unknown", path.clone(), None, format!("{label} does not say what kind of pack it is (pack.kind), so it cannot be inherited from")));
                continue;
            }
            Some(k) if k != ukind => {
                errs.push(Diagnostic::error("upstream_kind_mismatch", path.clone(), None, format!("{label} is a {k} pack, the pack says {ukind}")));
                continue;
            }
            Some(k) if !allowed.contains(&k) => {
                errs.push(Diagnostic::error("upstream_kind_not_allowed", path.clone(), None, format!("a {kind} pack cannot inherit from a {k} pack ({label}); allowed: {}", if allowed.is_empty() { "nothing".to_string() } else { allowed.join(", ") })));
                continue;
            }
            _ => {}
        }
        let inherit = u.get("inherit").cloned().unwrap_or(Value::Null);
        let mut taken = BTreeSet::new();
        let mut counts = Map::new();
        for (section, key, list_key, out) in [("devices", "id", "devices", &mut devices), ("peripheral_blocks", "id", "blocks", &mut blocks), ("regions", "id", "regions", &mut regions), ("rules", "id", "rules", &mut rules)] {
            let want = ids(&inherit, list_key);
            let have = by_id(up, section, key);
            for id in &want {
                match have.get(id.as_str()) {
                    None => errs.push(Diagnostic::error("inherit_unknown_id", format!("{path}.inherit.{list_key}"), Some(id.clone()), format!("{label} has no {list_key} entry '{id}'"))),
                    Some(obj) => {
                        out.push((*obj).clone());
                        if section == "rules" {
                            inherited_rule_ids.insert(id.clone(), label.clone());
                        }
                        provenance.insert(id.clone(), json!(label));
                        cited_sources(obj, &mut taken);
                    }
                }
            }
            counts.insert(list_key.to_string(), json!(want.len()));
        }
        // the documents the inherited things cite, with their digests, so the evidence can still be checked
        let up_sources = by_id(up, "sources", "source_id");
        for sid in &taken {
            match up_sources.get(sid.as_str()) {
                None => errs.push(Diagnostic::error("inherit_unknown_source", path.clone(), Some(sid.clone()), format!("inherited evidence cites {sid}, which {label} does not list"))),
                Some(src) => {
                    // the same document (same id, version and digest) that this pack cites itself is one source: the inherited
                    // evidence is pointed at it, and it applies to the inherited devices as well
                    match arr(derived, "sources").iter().find(|o| s(o, "document_id") == s(src, "document_id")) {
                        Some(own) if s(own, "sha256") == s(src, "sha256") && s(own, "version") == s(src, "version") => {
                            if s(own, "source_id") != s(src, "source_id") {
                                alias.insert(sid.clone(), s(own, "source_id").unwrap_or("").to_string());
                            }
                            for d in arr(src, "applies_to").iter().filter_map(Value::as_str) {
                                extra_applies.entry(s(own, "source_id").unwrap_or("").to_string()).or_default().insert(d.to_string());
                            }
                        }
                        Some(own) => errs.push(Diagnostic::error(
                            "source_conflict",
                            "$.sources",
                            s(own, "source_id").map(str::to_string),
                            format!("{} cites the document {} at another version or digest than {label}: the packs disagree about the document", s(own, "source_id").unwrap_or(""), s(src, "document_id").unwrap_or("")),
                        )),
                        None => sources.push((*src).clone()),
                    }
                }
            }
        }
        resolved_from.push(json!({"id": uid, "version": uver, "kind": ukind, "canonical_sha256": digest, "inherited": Value::Object(counts)}));
    }

    // a rule that replaces an inherited one: only a listed one, and only with the derived pack's own evidence
    let mut replaced: BTreeSet<String> = BTreeSet::new();
    for (i, r) in arr(derived, "rules").iter().enumerate() {
        let Some(target) = s(r, "overrides") else { continue };
        let id = s(r, "id").unwrap_or("").to_string();
        let path = format!("$.rules[{i}].overrides");
        if !inherited_rule_ids.contains_key(target) {
            errs.push(Diagnostic::error("override_of_uninherited", path, Some(id), format!("'{target}' is not an inherited rule: list it in pack.upstream[].inherit.rules, or write a rule of your own without `overrides`")));
            continue;
        }
        replaced.insert(target.to_string());
        let mut own = BTreeSet::new();
        cited_sources(&json!({"evidence": r.get("evidence").cloned().unwrap_or(json!([]))}), &mut own);
        if s(r, "modifiability") != Some("unknown") && !own.iter().any(|x| own_sources.contains(x)) {
            errs.push(Diagnostic::error("override_without_own_evidence", format!("$.rules[{i}].evidence"), Some(id), format!("a rule that overrides '{target}' needs evidence from a source of this pack: the evidence of the upstream is what is being replaced")));
        }
    }
    rules.retain(|r| s(r, "id").is_none_or(|id| !replaced.contains(id)));
    provenance.retain(|k, _| !replaced.contains(k));

    // what the derived pack adds must not silently collide with, or contradict, what it inherited
    let inherited_by_key: BTreeMap<(String, String), &Value> = rules.iter().map(|r| ((s(r, "kind").unwrap_or("").to_string(), canonical_json(r.get("subject").unwrap_or(&Value::Null))), r)).collect();
    for (i, r) in arr(derived, "rules").iter().enumerate() {
        if s(r, "overrides").is_some() {
            continue;
        }
        let id = s(r, "id").unwrap_or("");
        // a rule that says `unknown` states nothing: it cannot contradict what was inherited (an id clash is still one)
        let states_nothing = s(r, "modifiability") == Some("unknown");
        if inherited_rule_ids.contains_key(id) {
            errs.push(Diagnostic::error("id_clash", format!("$.rules[{i}].id"), Some(id.to_string()), format!("the id '{id}' is also an inherited rule: use another id, or `overrides` with evidence")));
            continue;
        }
        let key = (s(r, "kind").unwrap_or("").to_string(), canonical_json(r.get("subject").unwrap_or(&Value::Null)));
        if let Some(inh) = inherited_by_key.get(&key).filter(|_| !states_nothing) {
            if canonical_json(r.get("params").unwrap_or(&Value::Null)) != canonical_json(inh.get("params").unwrap_or(&Value::Null)) {
                errs.push(Diagnostic::error(
                    "inherit_conflict",
                    format!("$.rules[{i}]"),
                    Some(id.to_string()),
                    format!("it states the same thing as the inherited rule {} with other values: replace the inherited rule explicitly (`overrides`, with evidence of this pack) or record the disagreement as a `conflict` rule", s(inh, "id").unwrap_or("")),
                ));
            }
        }
    }

    // the same document can arrive from two upstreams, or already be a source of the derived pack
    let mut merged_sources: Vec<Value> = Vec::new();
    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    for src in arr(derived, "sources").iter().chain(sources.iter()) {
        let sid = s(src, "source_id").unwrap_or("").to_string();
        match seen.get(&sid) {
            None => {
                seen.insert(sid, src.clone());
                merged_sources.push(src.clone());
            }
            Some(prev) if canonical_json(prev) == canonical_json(src) => {}
            Some(_) => errs.push(Diagnostic::error("source_conflict", "$.sources", Some(sid.clone()), format!("source {sid} is defined twice with different content (version or digest): the packs disagree about the document"))),
        }
    }
    for d in &devices {
        if arr(derived, "devices").iter().any(|x| s(x, "id") == s(d, "id")) {
            errs.push(Diagnostic::error("id_clash", "$.devices", s(d, "id").map(str::to_string), "an inherited device has the id of a device of this pack"));
        }
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    if !alias.is_empty() {
        for v in devices.iter_mut().chain(blocks.iter_mut()).chain(regions.iter_mut()).chain(rules.iter_mut()) {
            realias(v, &alias);
        }
    }
    let device_ids: BTreeSet<String> = devices.iter().chain(arr(derived, "devices")).filter_map(|d| s(d, "id")).map(str::to_string).collect();
    let mut flat = derived.clone();
    let obj = flat.as_object_mut().expect("a pack is an object");
    let prepend = |own: Vec<Value>, mut inherited: Vec<Value>| {
        inherited.extend(own);
        inherited
    };
    let own = |k: &str| arr(derived, k).to_vec();
    obj.insert("devices".into(), Value::Array(prepend(own("devices"), devices)));
    if !blocks.is_empty() || obj.contains_key("peripheral_blocks") {
        obj.insert("peripheral_blocks".into(), Value::Array(prepend(own("peripheral_blocks"), blocks)));
    }
    if !regions.is_empty() || obj.contains_key("regions") {
        obj.insert("regions".into(), Value::Array(prepend(own("regions"), regions)));
    }
    if !rules.is_empty() || obj.contains_key("rules") {
        obj.insert("rules".into(), Value::Array(prepend(own("rules"), rules)));
    }
    // a merged document applies to the devices of this pack and to the inherited ones it was cited for (only those that are in the pack)
    for src in merged_sources.iter_mut() {
        if let Some(more) = s(src, "source_id").and_then(|id| extra_applies.get(id)) {
            let list = src["applies_to"].as_array_mut().expect("applies_to");
            for d in more.iter().filter(|d| device_ids.contains(*d)) {
                if !list.iter().any(|x| x.as_str() == Some(d.as_str())) {
                    list.push(json!(d));
                }
            }
        }
    }
    obj.insert("sources".into(), Value::Array(merged_sources));
    let meta = obj.get_mut("pack").and_then(Value::as_object_mut).expect("pack metadata");
    meta.remove("upstream");
    meta.insert("x_resolved_from".into(), Value::Array(resolved_from));
    meta.insert("x_inherited".into(), Value::Object(provenance));

    let after: Vec<Diagnostic> = hcp::check(&flat).into_iter().filter(|d| d.severity == crate::diag::Severity::Error).map(|mut d| {
        d.message = format!("after merging the upstream packs: {}", d.message);
        d
    }).collect();
    if !after.is_empty() {
        return Err(after);
    }
    Ok(flat)
}

/// Points the evidence of an inherited object at the source of the derived pack that is the same document.
fn realias(v: &mut Value, alias: &BTreeMap<String, String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::Array(ev)) = m.get_mut("evidence") {
                for e in ev.iter_mut() {
                    if let Some(new) = e.get("source_id").and_then(Value::as_str).and_then(|id| alias.get(id)) {
                        e["source_id"] = json!(new);
                    }
                }
            }
            for c in m.values_mut() {
                realias(c, alias);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(|c| realias(c, alias)),
        _ => {}
    }
}
