//! A fixed base design and the regions an AI may edit: the whitelist, the draft, the boundary checks.

mod common;
use common::*;
use fr_constraint::compile;
use fr_constraint::edit::{self, BaseFile, EditCheck};
use fr_constraint::hcp;
use fr_constraint::ir::Ir;
use fr_constraint::report::Lang;
use fr_constraint::snapshot::Snapshot;
use serde_json::{json, Value};

const ALL_OPS: [&str; 7] = ["add_component", "move_component", "remove_component", "connect", "disconnect", "add_track", "add_via"];

fn variant(f: impl FnOnce(&mut Value)) -> Ir {
    let mut p = example();
    f(&mut p);
    compile::compile(&p).unwrap_or_else(|d| panic!("the variant does not compile: {d:#?}"))
}

fn all_ops() -> Ir {
    variant(|p| p["base_design"]["editable_regions"][0]["allowed_ops"] = json!(ALL_OPS))
}

fn set_polygon(p: &mut Value, region: &str, poly: Value) {
    for g in p["regions"].as_array_mut().unwrap() {
        if g["id"] == region {
            g["polygon"] = poly.clone();
        }
    }
}

fn draft(ir: &Ir, changes: Value) -> Value {
    let a = edit::authorization(ir).expect("the pack has a base design");
    json!({
        "format": "fastroute-hcp-edit-draft", "draft_version": "1.0", "title": "test",
        "pack": {"id": ir.pack.id, "version": ir.pack.version, "sha256": ir.pack.sha256},
        "base": {"id": a.base_id, "binding_sha256": a.binding_sha256},
        "changes": changes
    })
}

/// The base files as they are in a working copy that matches the pack.
fn files_ok(ir: &Ir) -> Vec<BaseFile> {
    let a = edit::authorization(ir).unwrap();
    a.binding["files"].as_array().unwrap().iter().map(|f| BaseFile { path: f["path"].as_str().unwrap().into(), sha256: f["sha256"].as_str().map(str::to_string) }).collect()
}

fn run(ir: &Ir, d: &Value) -> EditCheck {
    edit::check_edit(ir, d, Some(&legal()), Some(&files_ok(ir))).unwrap_or_else(|e| panic!("{e:#?}"))
}

fn codes(c: &EditCheck) -> Vec<&str> {
    c.findings.iter().filter(|f| f.severity == "error").map(|f| f.code.as_str()).collect()
}

fn only(c: &EditCheck, code: &str) -> Vec<edit::EditFinding> {
    c.findings.iter().filter(|f| f.code == code).cloned().collect()
}

fn led_changes() -> Value {
    json!([
        {"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "value": "LED", "at": [10.0, 20.0], "reason": "status LED"},
        {"id": "CH-2", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "series", "ref": "R2", "value": "330", "at": [14.0, 20.0]},
        {"id": "CH-3", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "5", "net": "LED_DRIVE"},
        {"id": "CH-4", "op": "connect", "region": "REG-USER", "ref": "R2", "pin": "1", "net": "LED_DRIVE"},
        {"id": "CH-5", "op": "connect", "region": "REG-USER", "ref": "R2", "pin": "2", "net": "LED_A"},
        {"id": "CH-6", "op": "connect", "region": "REG-USER", "ref": "D1", "pin": "1", "net": "LED_A"},
        {"id": "CH-7", "op": "connect", "region": "REG-USER", "ref": "D1", "pin": "2", "net": "GND"},
        {"id": "CH-8", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 0.25, "points": [[14.0, 20.0], [20.0, 20.0], [20.0, 26.0]]}
    ])
}

// ---- the whitelist --------------------------------------------------------------------------------

#[test]
fn the_whitelist_is_computed_from_the_pack_and_leaves_out_protected_pins() {
    let ir = ir();
    let a = edit::authorization(&ir).unwrap();
    assert_eq!(a.regions.len(), 1);
    let r = &a.regions[0];
    assert_eq!((r.id.as_str(), r.allowed_blocks.clone(), r.allowed_ops.len()), ("REG-USER", vec!["BLK-LED".to_string()], 3));
    assert_eq!(r.pin_pool["DEV-FM100"], vec!["5".to_string()]);
    for pin in ["1", "2", "3", "4", "10", "11"] {
        assert!(a.protected["DEV-FM100"].contains_key(pin), "pin {pin} is protected");
    }
    assert!(a.immutable_components.contains("U1") && a.immutable_components.contains("J1"));
    assert!(a.fixed_regions.iter().any(|g| g.id == "REG-ANT"), "the antenna keep-out is fixed");
    // the same pack, the same whitelist, the same request
    let q1 = edit::edit_request(&ir, "add a status LED").unwrap();
    let q2 = edit::edit_request(&compile::compile(&example()).unwrap(), "add a status LED").unwrap();
    assert_eq!(serde_json::to_string(&q1).unwrap(), serde_json::to_string(&q2).unwrap());
    assert!(fr_constraint::jsonschema::validate(&edit::request_schema(), &q1).is_empty(), "{:#?}", fr_constraint::jsonschema::validate(&edit::request_schema(), &q1));
    assert_eq!(q1["authorization"]["regions"][0]["blocks"][0]["id"], "BLK-LED");
    assert_eq!(q1["pack"]["sha256"], json!(ir.pack.sha256));
}

#[test]
fn a_pool_that_lists_a_protected_pin_is_a_pack_error_and_a_region_without_operations_is_a_warning() {
    let mut p = example();
    p["base_design"]["editable_regions"][0]["pin_pool"] = json!([{"device": "DEV-FM100", "pins": ["5", "10", "3"]}]);
    p["base_design"]["editable_regions"][0].as_object_mut().unwrap().remove("allowed_ops");
    let d = hcp::check(&p);
    let protected: Vec<_> = d.iter().filter(|x| x.code == "pool_pin_protected").collect();
    assert_eq!(protected.len(), 2, "{d:#?}");
    assert!(protected.iter().any(|x| x.object.as_deref() == Some("C-RESERVED")) && protected.iter().any(|x| x.object.as_deref() == Some("C-RESET")), "{protected:#?}");
    assert!(d.iter().any(|x| x.code == "no_allowed_ops" && x.severity == fr_constraint::diag::Severity::Warning));
}

#[test]
fn a_pack_without_a_base_design_has_nothing_editable() {
    let mut p = example();
    p.as_object_mut().unwrap().remove("base_design");
    // regions and blocks that were ai_editable would be refused by the pack check: make them fixed
    p["regions"].as_array_mut().unwrap().retain(|g| g["id"] != "REG-USER");
    p["peripheral_blocks"].as_array_mut().unwrap().retain(|b| b["id"] != "BLK-LED");
    let ir = compile::compile(&p).unwrap();
    assert!(edit::authorization(&ir).is_none());
    assert!(edit::edit_request(&ir, "anything").is_err());
    let d = json!({
        "format": "fastroute-hcp-edit-draft", "draft_version": "1.0",
        "pack": {"id": ir.pack.id, "version": ir.pack.version, "sha256": ir.pack.sha256},
        "base": {"id": "BASE-X", "binding_sha256": "0".repeat(64)},
        "changes": [{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [10.0, 20.0]}]
    });
    let c = edit::check_edit(&ir, &d, Some(&legal()), None).unwrap();
    assert_eq!(c.status, "rejected");
    assert!(codes(&c).contains(&"no_base_design"), "{:?}", codes(&c));
}

// ---- a draft inside the whitelist -----------------------------------------------------------------

#[test]
fn a_draft_inside_the_whitelist_is_accepted_and_shown_as_a_difference() {
    let ir = ir();
    let c = run(&ir, &draft(&ir, led_changes()));
    assert_eq!(c.status, "accepted", "{:#?}", c.findings);
    assert!(c.complete, "{:#?}", c.findings);
    assert_eq!(c.summary["accepted"], 8);
    assert_eq!(c.diff["components"]["added"].as_array().unwrap().len(), 2);
    assert_eq!(c.diff["tracks_added"], 1);
    let v = c.validation.as_ref().expect("the rules of the pack ran on the result");
    assert_eq!(v["new_failures"].as_array().unwrap().len(), 0, "{v}");
    assert_eq!(v["after"]["result"], "pass");
    // every accepted change names what authorises it
    let ch1 = &c.changes[0];
    assert!(ch1.authorized_by.as_ref().unwrap().contains("allowed_blocks: BLK-LED"), "{ch1:?}");
    assert!(!ch1.evidence.is_empty(), "the block's evidence travels with the change");
    assert!(c.changes[2].authorized_by.as_ref().unwrap().contains("pin_pool: DEV-FM100 pin 5"));
}

#[test]
fn the_report_shows_before_and_after_and_the_evidence_in_both_languages() {
    let ir = ir();
    let c = run(&ir, &draft(&ir, led_changes()));
    let zh = edit::edit_report(&c, &ir, Lang::Zh);
    assert!(zh.contains("AI 修改草案校验报告") && zh.contains("通过（草案在授权范围内）") && zh.contains("基板 + 草案") && zh.contains("CH-8"), "{zh}");
    let en = edit::edit_report(&c, &ir, Lang::En);
    assert!(en.contains("AI edit draft check report") && en.contains("accepted") && en.contains("add D1") && en.contains("BLK-LED"), "{en}");
    assert!(en.contains("R1 330 ohm in series with D1"), "the evidence of the block is quoted: {en}");
}

#[test]
fn the_check_is_deterministic_and_does_not_depend_on_any_search_result() {
    let ir = ir();
    let plain = draft(&ir, led_changes());
    let a = serde_json::to_string(&run(&ir, &plain)).unwrap();
    let b = serde_json::to_string(&run(&ir, &plain)).unwrap();
    assert_eq!(a, b);
    // a recall of the allowed block, with any score, changes nothing in the verdict of the changes
    let mut with = plain.clone();
    with["recall"] = json!([{"kind": "block", "id": "BLK-LED", "source": "vector", "score": 0.99}]);
    let (c1, c2) = (run(&ir, &plain), run(&ir, &with));
    assert_eq!(serde_json::to_string(&c1.changes).unwrap(), serde_json::to_string(&c2.changes).unwrap());
    assert_eq!((c1.status, c2.status), ("accepted", "accepted"));
}

// ---- the base and the pack are fixed --------------------------------------------------------------

#[test]
fn a_changed_base_file_or_pack_or_binding_needs_a_review() {
    let ir = ir();
    let d = draft(&ir, led_changes());
    // the file in the working copy is not the one the pack was made for
    let mut files = files_ok(&ir);
    files[0].sha256 = Some("f".repeat(64));
    let c = edit::check_edit(&ir, &d, Some(&legal()), Some(&files)).unwrap();
    assert_eq!(c.status, "needs_review");
    assert!(only(&c, "base_changed")[0].message.contains("carrier.kicad_pcb"), "{:#?}", c.findings);
    assert!(!c.complete && !c.review_reasons.is_empty());
    // the file is gone
    files[0].sha256 = None;
    assert!(edit::check_edit(&ir, &d, Some(&legal()), Some(&files)).unwrap().findings.iter().any(|f| f.code == "base_changed" && f.message.contains("missing")));
    // a draft made for another version of the pack
    let mut old = d.clone();
    old["pack"]["sha256"] = json!("a".repeat(64));
    let c = run(&ir, &old);
    assert_eq!(c.status, "needs_review");
    assert!(codes(&c).contains(&"pack_mismatch"));
    // a draft bound to another base
    let mut other = d.clone();
    other["base"]["binding_sha256"] = json!("b".repeat(64));
    assert!(codes(&run(&ir, &other)).contains(&"base_mismatch"));
    // the pack itself moved to another commit of the open board: the old draft no longer matches
    let moved = variant(|p| p["base_design"]["binding"]["repo"]["commit"] = json!("1".repeat(40)));
    let c = run(&moved, &d);
    assert!(codes(&c).contains(&"base_mismatch") && codes(&c).contains(&"pack_mismatch"), "{:?}", codes(&c));
    assert_eq!(c.status, "needs_review");
}

#[test]
fn the_digest_of_the_design_itself_is_compared_and_a_missing_comparison_is_said() {
    let ir = ir();
    let d = draft(&ir, led_changes());
    let a = edit::authorization(&ir).unwrap();
    let bound = a.binding["files"][0]["sha256"].as_str().unwrap().to_string();
    // a design that is the bound file
    let same = changed(|v| v["project"]["source_sha256"] = json!(bound));
    let c = edit::check_edit(&ir, &d, Some(&same), None).unwrap();
    assert_eq!((c.status, c.complete, c.base["verified"].as_bool()), ("accepted", true, Some(true)), "{:#?}", c.findings);
    // a design that is another file
    let other = changed(|v| v["project"]["source_sha256"] = json!("c".repeat(64)));
    let c = edit::check_edit(&ir, &d, Some(&other), None).unwrap();
    assert_eq!(c.status, "needs_review");
    assert!(codes(&c).contains(&"base_changed"));
    // no digest at all: accepted, and not complete
    let c = edit::check_edit(&ir, &d, Some(&legal()), None).unwrap();
    assert_eq!((c.status, c.complete), ("accepted", false));
    assert_eq!(only(&c, "base_unverified").len(), 1);
}

// ---- only what is whitelisted ---------------------------------------------------------------------

#[test]
fn a_block_that_a_search_found_is_not_authorised_with_or_without_the_search_result() {
    let ir = ir();
    let changes = json!([{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-RESET", "role": "pull_up", "ref": "R9", "value": "10k", "at": [10.0, 20.0]}]);
    let plain = run(&ir, &draft(&ir, changes.clone()));
    assert_eq!(plain.status, "rejected");
    assert_eq!(codes(&plain), vec!["unauthorized_block"]);
    // the same draft, with the block as a vector hit with a high score
    let mut d = draft(&ir, changes.clone());
    d["recall"] = json!([{"kind": "block", "id": "BLK-RESET", "source": "vector", "score": 0.97, "note": "similar circuit"}]);
    let c = run(&ir, &d);
    assert_eq!(c.status, "rejected");
    assert_eq!(codes(&c), vec!["unauthorized_block"]);
    assert!(only(&c, "unauthorized_block")[0].message.contains("a search result does not authorise anything"), "{:#?}", c.findings);
    assert_eq!(only(&c, "recall_not_authorized").len(), 1);
    // a block that exists nowhere in the pack
    let c = run(&ir, &draft(&ir, json!([{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-MADEUP", "role": "x", "ref": "R9", "at": [10.0, 20.0]}])));
    assert!(only(&c, "unauthorized_block")[0].message.contains("does not define"));
}

#[test]
fn a_draft_cannot_grant_itself_more_than_the_pack() {
    let ir = ir();
    let mut d = draft(&ir, led_changes());
    d["claims"] = json!({"regions": ["REG-USER", "REG-ANT"], "blocks": ["BLK-LED", "BLK-RESET"], "pins": [{"device": "DEV-FM100", "pins": ["5", "10"]}]});
    let c = run(&ir, &d);
    assert_eq!(c.status, "rejected");
    let f = only(&c, "widened_authorization");
    assert_eq!(f.len(), 1);
    for want in ["region REG-ANT", "block BLK-RESET", "pin DEV-FM100.10"] {
        assert!(f[0].message.contains(want), "{}", f[0].message);
    }
    // claims that are inside the whitelist are fine
    let mut ok = draft(&ir, led_changes());
    ok["claims"] = json!({"regions": ["REG-USER"], "blocks": ["BLK-LED"], "pins": [{"device": "DEV-FM100", "pins": ["5"]}]});
    assert_eq!(run(&ir, &ok).status, "accepted");
    // a whitelist of its own is not even a draft
    let mut own = draft(&ir, led_changes());
    own["allowed_blocks"] = json!(["BLK-RESET"]);
    own["pin_pool"] = json!(["10"]);
    let e = edit::check_edit(&ir, &own, Some(&legal()), None).unwrap_err();
    assert!(e.iter().any(|d| d.code == "schema"), "{e:#?}");
}

#[test]
fn reserved_strapping_reset_and_unlisted_pins_cannot_be_used() {
    let ir = ir();
    let c = |pin: &str| run(&ir, &draft(&ir, json!([{"id": "CH-1", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": pin, "net": "X"}])));
    // the flash pins: refused with the evidence of the rule that protects them
    let r = c("10");
    assert_eq!(codes(&r), vec!["reserved_pin_used"]);
    let f = &only(&r, "reserved_pin_used")[0];
    assert!(f.message.contains("C-RESERVED") && f.evidence[0].quote.contains("do not use"), "{f:?}");
    // reset, boot and ground are protected too
    for pin in ["3", "4", "1"] {
        assert_eq!(codes(&c(pin)), vec!["reserved_pin_used"], "pin {pin}");
    }
    // a free pin that the region does not list
    assert_eq!(codes(&c("6")), vec!["pin_not_in_pool"]);
    // the listed one
    assert_eq!(c("5").status, "accepted");
    // a pin that does not exist is not a pin of the pool either
    assert!(!codes(&c("99")).is_empty());
}

#[test]
fn nets_that_carry_protected_pins_are_not_joined() {
    let ir = ir();
    let mut ch = led_changes();
    ch[6] = json!({"id": "CH-7", "op": "connect", "region": "REG-USER", "ref": "D1", "pin": "2", "net": "EN"});
    let c = run(&ir, &draft(&ir, ch.clone()));
    assert_eq!(c.status, "rejected");
    let f = &only(&c, "protected_net")[0];
    assert!(f.message.contains("EN") && f.message.contains("C-RESET"), "{f:?}");
    ch[6]["net"] = json!("BOOT");
    assert!(codes(&run(&ir, &draft(&ir, ch))).contains(&"protected_net"));
    // supply and ground are not protected nets
    assert_eq!(run(&ir, &draft(&ir, led_changes())).status, "accepted");
}

#[test]
fn pins_already_in_use_are_not_taken() {
    let ir = variant(|p| p["base_design"]["editable_regions"][0]["pin_pool"] = json!([{"device": "DEV-FM100", "pins": ["5", "8"]}]));
    // pin 8 carries USB_DP to the connector
    let c = run(&ir, &draft(&ir, json!([{"id": "CH-1", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "8", "net": "SIG"}])));
    assert_eq!(codes(&c), vec!["pin_in_use"]);
    // and a pin that this draft took already
    let two = json!([
        {"id": "CH-1", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "5", "net": "SIG_A"},
        {"id": "CH-2", "op": "connect", "region": "REG-USER", "ref": "U1", "pin": "5", "net": "SIG_B"}
    ]);
    let c = run(&ir, &draft(&ir, two));
    assert_eq!((c.changes[0].verdict, c.changes[1].verdict), ("accepted", "rejected"));
    assert_eq!(codes(&c), vec!["pin_in_use"]);
}

#[test]
fn regions_operations_and_layers_are_limited_to_what_the_pack_lists() {
    let ir = ir();
    let one = |ch: Value| run(&ir, &draft(&ir, json!([ch])));
    // the antenna keep-out and an invented region are not editable
    for region in ["REG-ANT", "REG-NOPE"] {
        let c = one(json!({"id": "CH-1", "op": "add_via", "region": region, "net": "GND", "at": [10.0, 20.0], "diameter_mm": 0.6}));
        assert_eq!(codes(&c), vec!["region_not_editable"], "{region}");
    }
    // an operation the region does not list
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "add_via", "region": "REG-USER", "net": "GND", "at": [10.0, 20.0], "diameter_mm": 0.6}))), vec!["op_not_allowed"]);
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "remove_component", "region": "REG-USER", "ref": "C1"}))), vec!["op_not_allowed"]);
    // no list, no operation
    let none = variant(|p| p["base_design"]["editable_regions"][0].as_object_mut().unwrap().remove("allowed_ops").map(|_| ()).unwrap());
    assert_eq!(codes(&run(&none, &draft(&none, led_changes()))).iter().filter(|c| **c == "op_not_allowed").count(), 8);
    // a layer the region does not cover
    let top = variant(|p| {
        for g in p["regions"].as_array_mut().unwrap() {
            if g["id"] == "REG-USER" {
                g["layers"] = json!(["F.Cu"]);
            }
        }
    });
    let t = |layer: &str| run(&top, &draft(&top, json!([{"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": layer, "width_mm": 0.25, "points": [[10.0, 20.0], [12.0, 20.0]]}])));
    assert_eq!(t("F.Cu").status, "accepted");
    assert_eq!(codes(&t("B.Cu")), vec!["layer_not_allowed"]);
}

// ---- nothing crosses a border ---------------------------------------------------------------------

#[test]
fn copper_and_parts_stay_inside_the_region() {
    let ir = ir();
    let one = |ch: Value| run(&ir, &draft(&ir, json!([ch])));
    // a part outside the region, one on the board's far side
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [45.0, 20.0]}))), vec!["outside_region"]);
    // a track that leaves the region (the cross-region case)
    let c = one(json!({"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 0.25, "points": [[10.0, 20.0], [45.0, 20.0]]}));
    assert_eq!(c.status, "rejected");
    assert!(codes(&c).contains(&"crosses_region"), "{:?}", codes(&c));
    // a track whose end points are inside but whose copper touches the border
    let c = one(json!({"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 1.0, "points": [[5.2, 20.0], [10.0, 20.0]]}));
    assert_eq!(codes(&c), vec!["crosses_region"]);
    // a track around a notch: the end points are inside the L, the segment is not
    let l = variant(|p| set_polygon(p, "REG-USER", json!([[5, 15], [35, 15], [35, 25], [15, 25], [15, 38], [5, 38]])));
    let c = run(&l, &draft(&l, json!([{"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 0.25, "points": [[30.0, 20.0], [10.0, 35.0]]}])));
    assert!(codes(&c).contains(&"crosses_region"), "{:?}", codes(&c));
    // a part of one region cannot be wired from another
    let two = variant(|p| {
        p["regions"].as_array_mut().unwrap().push(json!({
            "id": "REG-USER2", "kind": "editable", "unit": "mm", "modifiability": "ai_editable", "polygon": [[37, 15], [58, 15], [58, 38], [37, 38]], "layers": ["all"],
            "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Mechanical drawing"}, "quote": "Second prototyping area.", "basis": "observed"}]
        }));
        p["base_design"]["editable_regions"].as_array_mut().unwrap().push(json!({"region": "REG-USER2", "allowed_blocks": ["BLK-LED"], "allowed_ops": ["add_component", "connect", "add_track"]}));
    });
    let c = run(&two, &draft(&two, json!([
        {"id": "CH-1", "op": "add_component", "region": "REG-USER2", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [45.0, 20.0]},
        {"id": "CH-2", "op": "connect", "region": "REG-USER", "ref": "D1", "pin": "1", "net": "LED_X"}
    ])));
    assert_eq!((c.changes[0].verdict, c.changes[1].verdict), ("accepted", "rejected"), "{:#?}", c.findings);
    assert_eq!(codes(&c), vec!["crosses_region"]);
    // and a track from one region into the other
    let c = run(&two, &draft(&two, json!([{"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_X", "layer": "F.Cu", "width_mm": 0.25, "points": [[30.0, 20.0], [45.0, 20.0]]}])));
    assert!(codes(&c).contains(&"crosses_region"));
}

#[test]
fn a_fixed_keepout_cannot_be_entered_even_from_inside_an_editable_region() {
    // an editable region that (by a mistake in the pack) overlaps the antenna keep-out
    let ir = variant(|p| set_polygon(p, "REG-USER", json!([[5, 5], [55, 5], [55, 38], [5, 38]])));
    let c = run(&ir, &draft(&ir, json!([
        {"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "LED_DRIVE", "layer": "F.Cu", "width_mm": 0.25, "points": [[30.0, 12.0], [45.0, 8.0]]},
        {"id": "CH-2", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [50.0, 5.5]},
        {"id": "CH-3", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D2", "at": [30.0, 20.0]}
    ])));
    assert_eq!((c.changes[0].verdict, c.changes[1].verdict, c.changes[2].verdict), ("rejected", "rejected", "accepted"), "{:#?}", c.findings);
    let f = only(&c, "enters_keepout");
    assert_eq!(f.len(), 2);
    assert!(f.iter().all(|x| x.message.contains("REG-ANT") && x.evidence[0].quote.contains("antenna")), "{f:#?}");
}

#[test]
fn parts_outside_the_board_are_refused() {
    let ir = variant(|p| set_polygon(p, "REG-USER", json!([[5, 15], [35, 15], [35, 45], [5, 45]])));
    let c = run(&ir, &draft(&ir, json!([{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [20.0, 42.0]}])));
    assert_eq!(codes(&c), vec!["outside_board"]);
}

// ---- fixed objects --------------------------------------------------------------------------------

#[test]
fn fixed_parts_pins_and_the_base_design_cannot_be_changed() {
    let ir = all_ops();
    let one = |ch: Value| run(&ir, &draft(&ir, json!([ch])));
    // the module and the connector are immutable
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "U1", "to": [20.0, 20.0]}))), vec!["immutable_object"]);
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "remove_component", "region": "REG-USER", "ref": "J1"}))), vec!["immutable_object"]);
    // a device's pins are not disconnected
    let imm = variant(|p| {
        p["base_design"]["immutable"]["components"] = json!(["J1"]);
        p["base_design"]["editable_regions"][0]["allowed_ops"] = json!(ALL_OPS);
    });
    assert_eq!(codes(&run(&imm, &draft(&imm, json!([{"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "U1", "to": [20.0, 20.0]}])))), vec!["base_object_modified"]);
    assert_eq!(codes(&run(&imm, &draft(&imm, json!([{"id": "CH-1", "op": "disconnect", "region": "REG-USER", "ref": "U1", "pin": "6"}])))), vec!["base_object_modified"]);
    // a part of the base design that is outside every region is read-only
    let base = changed(|v| v["components"].as_array_mut().unwrap().push(json!({"ref": "R9", "value": "1k", "x": 45.0, "y": 20.0})));
    let d = draft(&ir, json!([{"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "R9", "to": [20.0, 20.0]}]));
    let c = edit::check_edit(&ir, &d, Some(&base), Some(&files_ok(&ir))).unwrap();
    assert_eq!(codes(&c), vec!["base_object_modified"]);
    // a part that is not there
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "remove_component", "region": "REG-USER", "ref": "R77"}))), vec!["ref_unknown"]);
    // an added reference cannot replace a base part
    assert_eq!(codes(&one(json!({"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "C1", "at": [10.0, 20.0]}))), vec!["ref_exists"]);
    // the design is needed to judge a change to the base
    let d = draft(&ir, json!([{"id": "CH-1", "op": "remove_component", "region": "REG-USER", "ref": "C1"}]));
    let c = edit::check_edit(&ir, &d, None, Some(&files_ok(&ir))).unwrap();
    assert_eq!(codes(&c), vec!["base_required"]);
    // the identity of a part is checked when the draft gives it
    let base = changed(|v| v["components"][1]["uuid"] = json!("11111111-aaaa"));
    let ok = draft(&ir, json!([{"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "C1", "uuid": "11111111-aaaa", "to": [20.0, 24.0]}]));
    assert_eq!(edit::check_edit(&ir, &ok, Some(&base), Some(&files_ok(&ir))).unwrap().status, "accepted");
    let stale = draft(&ir, json!([{"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "C1", "uuid": "22222222-bbbb", "to": [20.0, 24.0]}]));
    assert_eq!(codes(&edit::check_edit(&ir, &stale, Some(&base), Some(&files_ok(&ir))).unwrap()), vec!["stale_reference"]);
}

#[test]
fn parts_of_the_base_inside_a_region_can_be_changed_when_the_region_allows_it() {
    let ir = all_ops();
    let c = run(&ir, &draft(&ir, json!([
        {"id": "CH-1", "op": "move_component", "region": "REG-USER", "ref": "C3", "to": [20.0, 33.0]},
        {"id": "CH-2", "op": "disconnect", "region": "REG-USER", "ref": "C3", "pin": "2"},
        {"id": "CH-3", "op": "connect", "region": "REG-USER", "ref": "C3", "pin": "2", "net": "GND"},
        {"id": "CH-4", "op": "add_via", "region": "REG-USER", "net": "GND", "at": [10.0, 30.0], "diameter_mm": 0.6},
        {"id": "CH-5", "op": "remove_component", "region": "REG-USER", "ref": "SW1"}
    ])));
    assert!(c.changes.iter().take(4).all(|r| r.verdict == "accepted"), "{:#?}", c.findings);
    assert_eq!(c.diff["components"]["moved"][0]["from"]["at"], json!([27.0, 30.0]));
    assert_eq!(c.diff["components"]["moved"][0]["to"]["at"], json!([20.0, 33.0]));
    // removing the boot button breaks a rule of the pack, and the validator says which
    assert_eq!(c.status, "rejected");
    assert!(c.findings.iter().any(|f| f.code == "rule_violation" && f.message.contains("C-NEED-BOOT")), "{:#?}", c.findings);
}

// ---- the rules of the pack run on the result ------------------------------------------------------

#[test]
fn a_change_inside_the_whitelist_that_breaks_a_rule_is_rejected_by_the_validator() {
    let ir = ir();
    // lengthening one half of the USB pair breaks its skew limit
    let c = run(&ir, &draft(&ir, json!([{"id": "CH-1", "op": "add_track", "region": "REG-USER", "net": "USB_DP", "layer": "F.Cu", "width_mm": 0.2, "points": [[10.0, 20.0], [30.0, 20.0]]}])));
    assert_eq!(c.status, "rejected");
    let f = only(&c, "rule_violation");
    assert_eq!(f.len(), 1, "{:#?}", c.findings);
    assert!(f[0].message.contains("C-USB") && !f[0].evidence.is_empty());
    assert_eq!(c.validation.as_ref().unwrap()["new_failures"][0]["rule_id"], "C-USB");
    // the change itself was within the whitelist
    assert_eq!(c.changes[0].verdict, "accepted");
    // a design that already fails is not blamed on the draft
    let bad = changed(|v| v["tracks"][0]["x2"] = json!(60.0));
    let d = draft(&ir, led_changes());
    let c = edit::check_edit(&ir, &d, Some(&bad), Some(&files_ok(&ir))).unwrap();
    assert_eq!(c.status, "accepted", "{:#?}", c.findings);
    assert_eq!(c.validation.as_ref().unwrap()["already_failing"], 1);
}

#[test]
fn without_a_design_only_the_whitelist_is_checked_and_it_says_so() {
    let ir = ir();
    let d = draft(&ir, json!([{"id": "CH-1", "op": "add_component", "region": "REG-USER", "block": "BLK-LED", "role": "led", "ref": "D1", "at": [10.0, 20.0]}]));
    let c = edit::check_edit(&ir, &d, None, Some(&files_ok(&ir))).unwrap();
    assert_eq!((c.status, c.complete), ("accepted", false));
    assert_eq!(only(&c, "validator_not_run").len(), 1);
    assert!(c.validation.is_none());
}

// ---- stopping, emptiness, invalid drafts ----------------------------------------------------------

#[test]
fn a_draft_that_stops_reports_what_is_missing_and_may_not_carry_changes() {
    let ir = ir();
    let mut d = draft(&ir, json!([]));
    d["stop"] = json!({"reason": "the LED needs a 5 V rail, which no block provides", "missing": ["a boost block"]});
    let c = run(&ir, &d);
    assert_eq!(c.status, "stopped");
    assert!(only(&c, "stopped")[0].message.contains("a boost block"));
    d["changes"] = led_changes();
    let c = run(&ir, &d);
    assert_eq!(c.status, "rejected");
    assert!(codes(&c).contains(&"stop_with_changes"));
    // an empty draft is a draft that does nothing
    let c = run(&ir, &draft(&ir, json!([])));
    assert_eq!((c.status, only(&c, "empty_draft").len()), ("accepted", 1));
}

#[test]
fn a_malformed_draft_is_not_checked_at_all() {
    let ir = ir();
    let bad = |f: &dyn Fn(&mut Value)| {
        let mut d = draft(&ir, led_changes());
        f(&mut d);
        edit::check_edit(&ir, &d, Some(&legal()), None).unwrap_err()
    };
    assert!(bad(&|d| { d.as_object_mut().unwrap().remove("pack"); }).iter().any(|x| x.code == "schema" && x.message.contains("pack")));
    assert!(bad(&|d| d["changes"][0]["op"] = json!("rewrite_board")).iter().any(|x| x.code == "schema"));
    assert!(bad(&|d| d["changes"][0]["unexpected"] = json!(1)).iter().any(|x| x.code == "schema"));
    assert!(bad(&|d| d["changes"][7]["width_mm"] = json!(0)).iter().any(|x| x.code == "schema"));
    assert!(bad(&|d| d["changes"][1]["id"] = json!("CH-1")).iter().any(|x| x.code == "duplicate_change_id"));
    assert!(bad(&|d| d["changes"][0]["region"] = json!("anywhere")).iter().any(|x| x.code == "schema"));
    // an IR that was edited by hand is refused
    let mut tampered = ir.clone();
    tampered.rules[0].title = Some("changed".into());
    let e = edit::check_edit(&tampered, &draft(&ir, led_changes()), Some(&legal()), None).unwrap_err();
    assert_eq!(e[0].code, "ir_integrity");
}

#[test]
fn the_schema_of_the_draft_and_the_request_are_valid_documents() {
    let ir = ir();
    let d = draft(&ir, led_changes());
    assert!(fr_constraint::jsonschema::validate(&edit::draft_schema(), &d).is_empty());
    let s: Snapshot = legal();
    assert_eq!(s.components.len(), 7);
}

#[test]
fn the_published_example_draft_matches_the_published_example_pack_and_is_accepted() {
    let ir = ir();
    let d: Value = serde_json::from_str(&std::fs::read_to_string(skill().join("assets/example-edit-draft.json")).unwrap()).unwrap();
    let a = edit::authorization(&ir).unwrap();
    assert_eq!(
        (d["pack"]["sha256"].as_str(), d["base"]["binding_sha256"].as_str()),
        (Some(ir.pack.sha256.as_str()), Some(a.binding_sha256.as_str())),
        "the example draft is for another version of the example pack: write it again from `fastroute constraints edit-request`"
    );
    let c = edit::check_edit(&ir, &d, Some(&legal()), Some(&files_ok(&ir))).unwrap();
    assert_eq!((c.status, c.complete), ("accepted", true), "{:#?}", c.findings);
}
