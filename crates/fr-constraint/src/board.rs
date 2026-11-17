//! A ready-made development board embedded in a new carrier as a fixed module (`pack.kind: board`, `board_integration`, hcp_version 1.1).
//!
//! The board pack says what the board is: its outline and keep-outs in its own frame (`regions` with frame `device`), the header pins it
//! brings out (only those are pins of the board: pins of the chip that are not brought out are in no pool), which of them may be used,
//! what the board implements itself, and what a carrier may add. A request says which board (exact pack, digest and hardware revision),
//! the carrier outline (a rectangle) and where the board sits. [`embed`] turns the two into
//!
//! * a **carrier pack**: an ordinary, flat HCP document in the carrier's frame with a `base_design` that the existing edit boundary
//!   (`edit-request`, `check-edit`) reads: the board is an immutable component, its outline and keep-outs are fixed regions, the free
//!   area of the carrier is cut into editable regions, and the pin pool is the header pins that are usable; and
//! * a **result**: the board in the carrier's frame, the pool and every pin that is not in it with the reason, the checked targets, what
//!   is unknown.
//!
//! Everything is deterministic (the same pack and request give the same bytes) and needs no network, no AI and no index. A target or
//! a similarity from a search widens nothing: the pool is computed from the pack.

use crate::diag::Diagnostic;
use crate::hcp::{canonical_json, sha256_hex};
use crate::jsonschema;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

pub const REQUEST_SCHEMA_TEXT: &str = include_str!("../../../skills/hardware-constraint-design/schemas/board-embedding-request-v1.schema.json");
pub const RESULT_FORMAT: &str = "hcp-board-embedding";

pub fn request_schema() -> Value {
    serde_json::from_str(REQUEST_SCHEMA_TEXT).expect("the embedded request schema is valid JSON")
}

pub struct Embedding {
    /// The embedding result (`format: hcp-board-embedding`)
    pub result: Value,
    /// The flat HCP document of the embedded design, in the carrier's frame
    pub carrier_pack: Value,
}

type R = (f64, f64, f64, f64); // x0, y0, x1, y1

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str)
}

fn arr<'a>(v: &'a Value, k: &str) -> &'a [Value] {
    v.get(k).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn pt(v: &Value) -> [f64; 2] {
    [num(&v[0]), num(&v[1])]
}

/// Rotates counter-clockwise about the origin by a multiple of 90 degrees and moves the origin to `at`.
fn place(p: [f64; 2], rot: i64, at: [f64; 2]) -> [f64; 2] {
    let (x, y) = match rot {
        90 => (-p[1], p[0]),
        180 => (-p[0], -p[1]),
        270 => (p[1], -p[0]),
        _ => (p[0], p[1]),
    };
    [r6(x + at[0]), r6(y + at[1])]
}

/// Rounded to a millionth of a millimetre, so that the same input is the same text on every machine.
fn r6(x: f64) -> f64 {
    let v = (x * 1e6).round() / 1e6;
    if v == 0.0 {
        0.0
    } else {
        v
    }
}

fn bbox(poly: &[[f64; 2]]) -> R {
    let xs = poly.iter().map(|p| p[0]);
    let ys = poly.iter().map(|p| p[1]);
    (xs.clone().fold(f64::INFINITY, f64::min), ys.clone().fold(f64::INFINITY, f64::min), xs.fold(f64::NEG_INFINITY, f64::max), ys.fold(f64::NEG_INFINITY, f64::max))
}

fn rect_poly(r: R) -> Value {
    json!([[r6(r.0), r6(r.1)], [r6(r.2), r6(r.1)], [r6(r.2), r6(r.3)], [r6(r.0), r6(r.3)]])
}

/// The area of `carrier` not covered by any blocker, as rectangles: the carrier is cut into vertical slabs at every edge of a blocker,
/// each slab has free stretches, and a stretch that continues unchanged into the next slab is one rectangle. The rectangles do not
/// overlap and together are exactly the free area (slivers narrower than `min` are dropped, and counted).
fn free_rects(carrier: R, blockers: &[R], min: f64) -> (Vec<R>, usize) {
    let mut xs: Vec<f64> = vec![carrier.0, carrier.2];
    for b in blockers {
        for x in [b.0, b.2] {
            if x > carrier.0 && x < carrier.2 {
                xs.push(x);
            }
        }
    }
    xs.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    xs.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    let mut done: Vec<R> = Vec::new();
    let mut open: Vec<R> = Vec::new(); // rectangles that end at the current slab's left edge
    for w in xs.windows(2) {
        let (xa, xb) = (w[0], w[1]);
        let mut cover: Vec<(f64, f64)> = blockers.iter().filter(|b| b.0 < xb - 1e-9 && b.2 > xa + 1e-9).map(|b| (b.1.max(carrier.1), b.3.min(carrier.3))).filter(|c| c.1 > c.0).collect();
        cover.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
        let mut merged: Vec<(f64, f64)> = Vec::new();
        for c in cover {
            match merged.last_mut() {
                Some(l) if c.0 <= l.1 + 1e-9 => l.1 = l.1.max(c.1),
                _ => merged.push(c),
            }
        }
        let mut free = Vec::new();
        let mut y = carrier.1;
        for c in &merged {
            if c.0 > y + 1e-9 {
                free.push((y, c.0));
            }
            y = y.max(c.1);
        }
        if carrier.3 > y + 1e-9 {
            free.push((y, carrier.3));
        }
        let mut next: Vec<R> = Vec::new();
        for (ya, yb) in free {
            match open.iter().position(|r| (r.1 - ya).abs() < 1e-9 && (r.3 - yb).abs() < 1e-9 && (r.2 - xa).abs() < 1e-9) {
                Some(i) => {
                    let mut r = open.remove(i);
                    r.2 = xb;
                    next.push(r);
                }
                None => next.push((xa, ya, xb, yb)),
            }
        }
        done.append(&mut open);
        open = next;
    }
    done.append(&mut open);
    let mut out = Vec::new();
    let mut dropped = 0;
    for r in done {
        if r.2 - r.0 >= min && r.3 - r.1 >= min {
            out.push(r);
        } else {
            dropped += 1;
        }
    }
    out.sort_by(|a, b| (a.1, a.0).partial_cmp(&(b.1, b.0)).expect("finite"));
    (out, dropped)
}

fn err(code: &str, path: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::error(code, path, None, message)
}

/// Embeds the board of `board` (the HCP document, already resolved with its upstream packs and valid) in the carrier of `request`.
/// `bound_sha256` is the canonical digest of the board pack's document as the request has to name it.
pub fn embed(board: &Value, bound_sha256: &str, request: &Value) -> Result<Embedding, Vec<Diagnostic>> {
    let schema_errors: Vec<Diagnostic> = jsonschema::validate(&request_schema(), request).into_iter().map(|e| err("request_schema", &e.path, e.message)).collect();
    if !schema_errors.is_empty() {
        return Err(schema_errors);
    }
    let Some(bi) = board.get("board_integration") else {
        return Err(vec![err("no_board_integration", "$", "this pack has no board_integration: it is not a development board that can be embedded")]);
    };
    let meta = &board["pack"];
    let mut errs = Vec::new();

    // the request is for exactly this pack, this version, this hardware revision and this content
    let rb = &request["board"];
    if s(rb, "id") != s(meta, "id") || s(rb, "version") != s(meta, "version") {
        errs.push(err("board_mismatch", "$.board", format!("the request is for {}@{}, the pack is {}@{}", s(rb, "id").unwrap_or("?"), s(rb, "version").unwrap_or("?"), s(meta, "id").unwrap_or("?"), s(meta, "version").unwrap_or("?"))));
    }
    if s(rb, "hardware_revision") != s(bi, "hardware_revision") {
        errs.push(err(
            "board_revision_mismatch",
            "$.board.hardware_revision",
            format!("the request is for hardware revision {}, this pack describes revision {}: another revision of a board is another pack (its headers, parts and open design files may differ)", s(rb, "hardware_revision").unwrap_or("?"), s(bi, "hardware_revision").unwrap_or("?")),
        ));
    }
    if s(rb, "canonical_sha256") != Some(bound_sha256) {
        errs.push(err("board_pack_changed", "$.board.canonical_sha256", format!("the pack has the canonical digest {bound_sha256}, the request names {}: the pack changed after the request was made, so it has to be looked at again", s(rb, "canonical_sha256").unwrap_or("?"))));
    }
    if !errs.is_empty() {
        return Err(errs);
    }

    let dev_id = s(bi, "board_device").unwrap_or("").to_string();
    let rot = request["placement"]["rotation_deg"].as_i64().unwrap_or(0);
    let at = pt(&request["placement"]["origin_mm"]);
    let o = &request["carrier"]["outline"];
    let carrier: R = (num(&o["x_mm"]), num(&o["y_mm"]), num(&o["x_mm"]) + num(&o["width_mm"]), num(&o["y_mm"]) + num(&o["height_mm"]));
    let margin = request["editable"]["margin_mm"].as_f64().or_else(|| bi["carrier"]["margin_mm"].as_f64()).unwrap_or(0.0);
    let mut notes: Vec<String> = Vec::new();

    // the board's regions in the board's own frame -> the carrier's frame
    let own: Vec<&Value> = arr(board, "regions").iter().filter(|r| s(r, "frame") == Some("device") && s(r, "device") == Some(dev_id.as_str())).collect();
    let outline = own.iter().find(|r| s(r, "kind") == Some("board_outline")).copied().ok_or_else(|| vec![err("board_outline_missing", "$.regions", "the board has no outline")])?;
    let shape = |r: &Value| -> Vec<[f64; 2]> {
        if let Some(c) = r.get("circle") {
            // a circle is kept as its bounding square for the free-area computation; the region itself stays a circle
            let (cx, cy, rad) = (num(&c["center"][0]), num(&c["center"][1]), num(&c["radius_mm"]));
            return vec![place([cx - rad, cy - rad], rot, at), place([cx + rad, cy + rad], rot, at)];
        }
        arr(r, "polygon").iter().map(|p| place(pt(p), rot, at)).collect()
    };
    let body_poly: Vec<[f64; 2]> = shape(outline);
    let body = bbox(&body_poly);
    if body_poly.len() != 4 || !is_axis_aligned(&body_poly) {
        notes.push("the board outline is not a rectangle: its bounding rectangle is used as the body (conservative)".into());
    }

    // header pins in the carrier's frame; each must land on the carrier
    let mut headers_out = Vec::new();
    let mut pool: Vec<String> = Vec::new();
    let mut excluded = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    let mut all_pins: Vec<String> = Vec::new();
    let mut errs = Vec::new();
    let mut reasons: std::collections::BTreeMap<String, (String, String)> = Default::default(); // pin -> (usability, reason)
    for h in arr(bi, "exposed_headers") {
        let hid = s(h, "id").unwrap_or("");
        let mut pins_out = Vec::new();
        for p in arr(h, "pins") {
            let number = s(p, "number").unwrap_or("");
            let id = format!("{hid}.{number}");
            let c = place(pt(&p["position"]), rot, at);
            if c[0] < carrier.0 - 1e-9 || c[0] > carrier.2 + 1e-9 || c[1] < carrier.1 - 1e-9 || c[1] > carrier.3 + 1e-9 {
                errs.push(err("header_outside_carrier", "$.placement", format!("header pin {id} lands at ({}, {}), outside the carrier outline: the board cannot be plugged in there", c[0], c[1])));
            }
            let usability = s(p, "usability").unwrap_or("unknown");
            let reason = s(p, "reason").unwrap_or("");
            all_pins.push(id.clone());
            match usability {
                "usable" => pool.push(id.clone()),
                other => {
                    excluded.push(json!({"pin": id, "name": p["name"], "usability": other, "reason": reason}));
                    reasons.insert(id.clone(), (other.to_string(), reason.to_string()));
                    if other == "unknown" {
                        unknown.push(format!("{id} ({}): {reason}", s(p, "name").unwrap_or("")));
                    }
                }
            }
            pins_out.push(json!({"id": id, "number": number, "name": p["name"], "kind": p["kind"], "position_mm": [c[0], c[1]], "usability": usability}));
        }
        headers_out.push(json!({"id": hid, "pins": pins_out}));
    }
    if !errs.is_empty() {
        return Err(errs);
    }
    pool.sort();

    // fixed objects on the carrier: the board body and what the board says may not be built over
    let mut fixed_regions: Vec<Value> = Vec::new();
    let mut blockers: Vec<R> = vec![(body.0 - margin, body.1 - margin, body.2 + margin, body.3 + margin)];
    let mut immutable_regions: Vec<String> = vec!["REG-BOARD-BODY".to_string()];
    let mut has_holes = false;
    let outline_ev = outline.get("evidence").cloned().unwrap_or(json!([]));
    fixed_regions.push(json!({
        "id": "REG-BOARD-BODY", "title": format!("body of the board {}", s(meta, "id").unwrap_or("")), "kind": "module_body", "unit": "mm", "frame": "board",
        "polygon": rect_poly(body), "modifiability": "fixed", "evidence": outline_ev
    }));
    for r in &own {
        let kind = s(r, "kind").unwrap_or("");
        if kind == "board_outline" {
            continue;
        }
        let id = s(r, "id").unwrap_or("");
        let mut g = Map::new();
        for k in ["id", "title", "kind", "unit", "layers", "modifiability", "evidence", "missing"] {
            if let Some(v) = r.get(k) {
                g.insert(k.into(), v.clone());
            }
        }
        g.insert("frame".into(), json!("board"));
        if r.get("circle").is_some() {
            let c = &r["circle"];
            g.insert("circle".into(), json!({"center": place(pt(&c["center"]), rot, at), "radius_mm": c["radius_mm"]}));
        } else {
            g.insert("polygon".into(), json!(arr(r, "polygon").iter().map(|p| place(pt(p), rot, at)).collect::<Vec<_>>()));
        }
        if kind == "mount_hole" {
            has_holes = true;
        }
        if kind != "mount_hole" {
            let poly = shape(r);
            if !(poly.len() == 4 && is_axis_aligned(&poly)) {
                notes.push(format!("{id}: not a rectangle: its bounding rectangle keeps the free area away"));
            }
            blockers.push(bbox(&poly));
        }
        immutable_regions.push(id.to_string());
        fixed_regions.push(Value::Object(g));
    }

    // what is left of the carrier is the editable area
    let (free, dropped) = free_rects(carrier, &blockers, 0.5);
    if dropped > 0 {
        notes.push(format!("{dropped} sliver(s) of free area narrower than 0.5 mm are not editable"));
    }
    if free.is_empty() {
        notes.push("the board and its keep-outs cover the whole carrier: there is no area a circuit may be added in".into());
    }
    let req_sha = sha256_hex(canonical_json(request).as_bytes());
    let name = s(&request["carrier"], "name").unwrap_or("carrier");
    let date = s(request, "date").or_else(|| s(meta, "created")).unwrap_or("1970-01-01").to_string();

    // the sources: the board's, and the request itself as an observation of the carrier
    let mut sources: Vec<Value> = arr(board, "sources").iter().filter(|x| arr(x, "applies_to").iter().any(|d| d.as_str() == Some(dev_id.as_str()))).cloned().collect();
    sources.push(json!({
        "source_id": "SRC-EMBED-REQUEST", "document_id": "DOC-EMBED-REQUEST", "title": format!("board embedding request of the carrier {name}"), "version": &req_sha[..8], "level": "community",
        "retrieved": date, "sha256": req_sha, "applies_to": [dev_id]
    }));
    let req_ev = |locator: &str, quote: String| json!([{"source_id": "SRC-EMBED-REQUEST", "locator": {"section": locator}, "quote": quote, "basis": "observed"}]);

    let mut regions: Vec<Value> = vec![json!({
        "id": "REG-CARRIER-OUTLINE", "title": format!("outline of the carrier {name}"), "kind": "board_outline", "unit": "mm", "frame": "board", "polygon": rect_poly(carrier),
        "modifiability": "fixed", "evidence": req_ev("carrier.outline", canonical_json(&request["carrier"]["outline"]))
    })];
    regions.extend(fixed_regions);
    let mut editable = Vec::new();
    let allowed_blocks: Vec<String> = arr(&bi["carrier"], "allowed_blocks").iter().filter_map(Value::as_str).map(str::to_string).collect();
    let allowed_ops: Vec<String> = arr(&bi["carrier"], "allowed_ops").iter().filter_map(Value::as_str).map(str::to_string).collect();
    if allowed_ops.is_empty() {
        notes.push("the pack lists no allowed operations for the carrier: nothing may be done in the editable areas until it does".into());
    }
    for (i, f) in free.iter().enumerate() {
        let id = format!("REG-CARRIER-FREE-{}", i + 1);
        regions.push(json!({
            "id": id, "title": format!("free area {} of the carrier {name}", i + 1), "kind": "editable", "unit": "mm", "frame": "board", "polygon": rect_poly(*f),
            "modifiability": "ai_editable", "evidence": req_ev("carrier.outline minus the board", format!("carrier {} minus the body and keep-outs of the board", canonical_json(&request["carrier"]["outline"])))
        }));
        editable.push(json!({"region": id, "allowed_blocks": allowed_blocks, "pin_pool": [{"device": dev_id, "pins": pool}], "allowed_ops": allowed_ops}));
    }

    // blocks: what the board implements (fixed) and what the carrier may add
    let onboard: Vec<String> = arr(bi, "onboard_blocks").iter().filter_map(Value::as_str).map(str::to_string).collect();
    let keep: BTreeSet<&str> = onboard.iter().chain(&allowed_blocks).map(String::as_str).collect();
    let blocks: Vec<Value> = arr(board, "peripheral_blocks").iter().filter(|b| s(b, "id").is_some_and(|i| keep.contains(i))).cloned().collect();
    let block_ids: BTreeSet<String> = blocks.iter().filter_map(|b| s(b, "id").map(str::to_string)).collect();
    // rules about the board device and about the blocks that came along; nothing about pins that the board does not bring out
    let rules: Vec<Value> = arr(board, "rules")
        .iter()
        .filter(|r| {
            let sub = r.get("subject").cloned().unwrap_or(Value::Null);
            s(&sub, "device") == Some(dev_id.as_str()) || s(&sub, "block").is_some_and(|b| block_ids.contains(b)) || s(&sub, "region").is_some_and(|g| immutable_regions.iter().any(|i| i == g))
        })
        .cloned()
        .collect();
    // a rule of the board that its documents leave unknown is as much an open question as an unknown pin: it is listed, and the embedding needs attention
    for r in &rules {
        if s(r, "modifiability") == Some("unknown") {
            unknown.push(format!("rule {} ({}): {}", s(r, "id").unwrap_or(""), s(r, "kind").unwrap_or(""), s(r, "missing").or(s(r, "title")).unwrap_or("")));
        }
    }
    let device: Vec<Value> = arr(board, "devices").iter().filter(|d| s(d, "id") == Some(dev_id.as_str())).cloned().collect();

    // the targets are checked, never granted
    let mut targets_out = Vec::new();
    let mut attention = false;
    for t in arr(request, "targets") {
        let mut why: Vec<String> = Vec::new();
        if let Some(b) = s(t, "block") {
            if onboard.iter().any(|o| o == b) {
                why.push(format!("{b} is implemented by the board itself and is not added again"));
            } else if !allowed_blocks.iter().any(|a| a == b) {
                why.push(format!("{b} is not a block that the pack allows on the carrier"));
            }
        }
        for p in arr(t, "pins").iter().filter_map(Value::as_str) {
            if pool.iter().any(|x| x == p) {
                continue;
            }
            match reasons.get(p) {
                Some((u, r)) => why.push(format!("{p} cannot be used ({u}): {r}")),
                None if all_pins.iter().any(|x| x == p) => why.push(format!("{p} cannot be used")),
                None => why.push(format!("{p} is not a pin that the board brings out: pins of the chip that are not on a header are not available")),
            }
        }
        if !why.is_empty() {
            attention = true;
        }
        targets_out.push(json!({"name": t["name"], "block": t.get("block").cloned().unwrap_or(Value::Null), "pins": t.get("pins").cloned().unwrap_or(json!([])), "status": if why.is_empty() { "feasible" } else { "blocked" }, "reasons": why}));
    }

    let mut base = json!({
        "id": format!("{}-embedded-{name}", s(meta, "id").unwrap_or("board").replace('/', "-")),
        "title": format!("the carrier {name} with the board {}", s(meta, "title").unwrap_or("")),
        "binding": {"files": [{"path": "board-embedding-request.json", "sha256": req_sha}]},
        "immutable": {"components": [request["placement"]["ref"]], "board_outline": true, "mount_holes": has_holes, "regions": immutable_regions},
        "editable_regions": editable,
    });
    if !has_holes {
        base["immutable"]["mount_holes"] = json!(false);
    }
    let mut pack = Map::new();
    pack.insert("hcp_version".into(), json!("1.1"));
    pack.insert(
        "pack".into(),
        json!({
            "id": format!("{}/embedded-{}", s(meta, "id").unwrap_or("board"), name.to_ascii_lowercase()), "kind": "board",
            "title": format!("{} in the carrier {name}", s(meta, "title").unwrap_or("board")), "version": s(meta, "version").unwrap_or("0.1.0"), "language": s(meta, "language").unwrap_or("en"), "created": date,
            "extractor": {"kind": "mixed", "tool": "hcp-manager packs embed"},
            "review": {"status": "draft", "note": "Generated from the board pack and the embedding request; nothing in it is reviewed. The board pack is pinned by digest in x_embedded."},
            "x_embedded": {"board": {"id": rb["id"], "version": rb["version"], "hardware_revision": rb["hardware_revision"], "canonical_sha256": bound_sha256}, "request_sha256": req_sha}
        }),
    );
    pack.insert("sources".into(), Value::Array(sources));
    pack.insert("devices".into(), Value::Array(device));
    if !blocks.is_empty() {
        pack.insert("peripheral_blocks".into(), Value::Array(blocks));
    }
    if !rules.is_empty() {
        pack.insert("rules".into(), Value::Array(rules));
    }
    pack.insert("regions".into(), Value::Array(regions.clone()));
    pack.insert("base_design".into(), base);
    let carrier_pack = Value::Object(pack);

    let region_summary: Vec<Value> = regions.iter().map(|r| json!({"id": r["id"], "kind": r["kind"], "editable": r["kind"] == "editable", "polygon": r.get("polygon").cloned().unwrap_or(Value::Null)})).collect();
    let result = json!({
        "format": RESULT_FORMAT, "format_version": "1.0",
        "status": if attention || !unknown.is_empty() || free.is_empty() { "needs_attention" } else { "ok" },
        "board": {"id": rb["id"], "version": rb["version"], "hardware_revision": rb["hardware_revision"], "canonical_sha256": bound_sha256, "device": dev_id},
        "carrier": {"name": name, "outline": request["carrier"]["outline"]},
        "placement": request["placement"],
        "request_sha256": req_sha,
        "board_in_carrier": {"body_mm": rect_poly(body), "headers": headers_out},
        "regions": region_summary,
        "pin_pool": pool,
        "excluded_pins": excluded,
        "onboard_blocks": onboard,
        "allowed_blocks": allowed_blocks,
        "allowed_ops": allowed_ops,
        "targets": targets_out,
        "unknown": unknown,
        "notes": notes,
        "carrier_pack_canonical_sha256": sha256_hex(canonical_json(&carrier_pack).as_bytes()),
        "provenance": {
            "from_the_board_pack": {"devices": 1, "blocks": block_ids.len(), "rules": carrier_pack["rules"].as_array().map(Vec::len).unwrap_or(0), "board_regions": own.len(), "header_pins": all_pins.len()},
            "generated_for_the_carrier": {"regions": regions.len() - own.len() + 1, "editable_regions": free.len(), "pin_pool": pool.len()},
        },
    });
    Ok(Embedding { result, carrier_pack })
}

fn is_axis_aligned(p: &[[f64; 2]]) -> bool {
    p.len() == 4 && (0..4).all(|i| (p[i][0] - p[(i + 1) % 4][0]).abs() < 1e-9 || (p[i][1] - p[(i + 1) % 4][1]).abs() < 1e-9)
}

/// A readable report of the embedding, in Chinese or English: what is fixed by the board, which header pins may be used, what the
/// board implements, what the carrier has still to connect, what is unknown, and where the evidence is.
pub fn report(result: &Value, carrier_pack: &Value, zh: bool) -> String {
    let t = |a: &'static str, b: &'static str| if zh { a } else { b };
    let mut o = String::new();
    let board = &result["board"];
    o.push_str(&format!("# {} {}\n\n", t("开发板嵌入报告", "Board embedding report"), board["id"].as_str().unwrap_or("")));
    o.push_str(&format!(
        "- {}: {} {} ({} {}), {} `{}`\n- {}: {}\n- {}: {} ({} {}°)\n- {}: **{}**\n\n",
        t("开发板", "board"),
        board["id"].as_str().unwrap_or(""),
        board["version"].as_str().unwrap_or(""),
        t("硬件版本", "hardware revision"),
        board["hardware_revision"].as_str().unwrap_or(""),
        t("包摘要", "pack digest"),
        &board["canonical_sha256"].as_str().unwrap_or("")[..16.min(board["canonical_sha256"].as_str().unwrap_or("").len())],
        t("载板", "carrier"),
        result["carrier"]["name"].as_str().unwrap_or(""),
        t("位置", "placement"),
        result["placement"]["ref"].as_str().unwrap_or(""),
        t("旋转", "rotation"),
        result["placement"]["rotation_deg"],
        t("结论", "status"),
        result["status"].as_str().unwrap_or(""),
    ));
    o.push_str(&format!("## {}\n\n", t("一、开发板固定部分（不可改写）", "1. What the board fixes (read-only)")));
    o.push_str(&format!("{}\n\n", t("开发板本体、排针、板框和下列禁区在载板上的位置固定，AI 不得移动、重布或覆盖：", "The board body, its headers, outline and the keep-outs below are fixed on the carrier; nothing may move, re-route or cover them:")));
    for r in result["regions"].as_array().into_iter().flatten().filter(|r| r["editable"] == false && r["id"] != "REG-CARRIER-OUTLINE") {
        o.push_str(&format!("- `{}` ({}) {}\n", r["id"].as_str().unwrap_or(""), r["kind"].as_str().unwrap_or(""), r["polygon"]));
    }
    o.push_str(&format!("\n## {}\n\n", t("二、可用排针", "2. Header pins that may be used")));
    let pool: Vec<&str> = result["pin_pool"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    o.push_str(&format!("{} {}: {}\n\n", pool.len(), t("个（开发板对外引出且有证据）", "(brought out by the board, with evidence)"), pool.join(", ")));
    o.push_str(&format!("## {}\n\n", t("三、不可用或未知的排针", "3. Header pins that may not be used, or are unknown")));
    for p in result["excluded_pins"].as_array().into_iter().flatten() {
        o.push_str(&format!("- `{}` {} — {}: {}\n", p["pin"].as_str().unwrap_or(""), p["name"].as_str().unwrap_or(""), p["usability"].as_str().unwrap_or(""), p["reason"].as_str().unwrap_or("")));
    }
    o.push_str(&format!("\n## {}\n\n", t("四、开发板已板载（载板不得重复添加）", "4. Already on the board (not added again on the carrier)")));
    for b in result["onboard_blocks"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        let title = carrier_pack["peripheral_blocks"].as_array().into_iter().flatten().find(|x| x["id"] == b).and_then(|x| x["title"].as_str()).unwrap_or("");
        o.push_str(&format!("- `{b}` {title}\n"));
    }
    o.push_str(&format!("\n## {}\n\n", t("五、载板待连接的外设", "5. What the carrier is to get")));
    if result["targets"].as_array().is_none_or(|a| a.is_empty()) {
        o.push_str(&format!("{}\n", t("（请求中没有目标外设）", "(the request has no targets)")));
    }
    for tg in result["targets"].as_array().into_iter().flatten() {
        o.push_str(&format!("- **{}** — {}", tg["name"].as_str().unwrap_or(""), tg["status"].as_str().unwrap_or("")));
        for r in tg["reasons"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            o.push_str(&format!("\n  - {r}"));
        }
        o.push('\n');
    }
    o.push_str(&format!("\n## {}\n\n{}: {} ", t("六、载板可编辑区域", "6. Editable areas of the carrier"), t("可添加的块", "blocks that may be added"), result["allowed_blocks"]));
    o.push_str(&format!("{}: {}\n\n", t("允许的操作", "allowed operations"), result["allowed_ops"]));
    for r in result["regions"].as_array().into_iter().flatten().filter(|r| r["editable"] == true) {
        o.push_str(&format!("- `{}` {}\n", r["id"].as_str().unwrap_or(""), r["polygon"]));
    }
    o.push_str(&format!("\n## {}\n\n", t("七、未知项与备注", "7. Unknown items and notes")));
    for u in result["unknown"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        o.push_str(&format!("- {}: {u}\n", t("未知", "unknown")));
    }
    for n in result["notes"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        o.push_str(&format!("- {n}\n"));
    }
    o.push_str(&format!("\n## {}\n\n{}\n", t("八、证据位置", "8. Where the evidence is"), t("每个区域和每个可用排针的证据（文档、版本、页码或章节、原文引用）都在生成的载板知识包中，开发板自身的证据来自开发板包所引用的锁定文档。", "The evidence of every region and every usable pin (document, version, page or section, quotation) is in the generated carrier pack; the board's own evidence comes from the locked documents its pack cites.")));
    o
}

/// What the carrier pack took from the board pack, what it generated, and what it left out (with the reason): the difference between the
/// two documents, section by section. Deterministic; the board pack is the resolved one `embed` read.
pub fn diff(board: &Value, carrier_pack: &Value) -> Value {
    let ids = |v: &Value, section: &str, key: &str| -> Vec<String> { arr(v, section).iter().filter_map(|x| s(x, key)).map(str::to_string).collect() };
    let section = |name: &str, key: &str, why_left_out: &str| {
        let (b, c) = (ids(board, name, key), ids(carrier_pack, name, key));
        let carried: Vec<&String> = b.iter().filter(|i| c.contains(i)).collect();
        let left_out: Vec<&String> = b.iter().filter(|i| !c.contains(i)).collect();
        let generated: Vec<&String> = c.iter().filter(|i| !b.contains(i)).collect();
        json!({"carried_over": carried, "generated": generated, "left_out": left_out, "why_left_out": if left_out.is_empty() { "" } else { why_left_out }})
    };
    json!({
        "format": "hcp-board-embedding-diff", "format_version": "1.0",
        "board_pack": board["pack"]["id"], "carrier_pack": carrier_pack["pack"]["id"],
        "sections": {
            "sources": section("sources", "source_id", "no fact of the carrier pack cites it"),
            "devices": section("devices", "id", "not the board itself: the module and the chip are not parts of the carrier design, and the pins of the chip that the board does not bring out are not pins of the board"),
            "peripheral_blocks": section("peripheral_blocks", "id", "neither implemented by the board nor allowed on the carrier"),
            "rules": section("rules", "id", "about a device or block that is not part of the carrier design; what they protect is already in the usability of the header pins"),
            "regions": section("regions", "id", "none"),
        },
        "regions_moved_to_the_carrier_frame": arr(board, "regions").iter().filter(|r| s(r, "frame") == Some("device")).filter_map(|r| s(r, "id")).collect::<Vec<_>>(),
        "base_design": {"added": carrier_pack.get("base_design").is_some() && board.get("base_design").is_none(), "editable_regions": arr(&carrier_pack["base_design"], "editable_regions").len()},
    })
}
