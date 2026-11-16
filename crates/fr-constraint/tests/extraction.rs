//! Extraction samples (the cases of the extraction guide): an explicit rule, a conditional rule, a
//! reference board that differs from the manual, missing data, a mistaken recall by a retrieval
//! tool, and a document of a similar model. Each is a pack that is accepted or refused, and what the
//! readable report says about it.

mod common;
use common::*;
use fr_constraint::compile;
use fr_constraint::hcp;
use fr_constraint::pack_report::pack_report;
use fr_constraint::report::Lang;
use fr_constraint::validate::{validate, Verdict};
use serde_json::{json, Value};

fn rule_mut<'a>(pack: &'a mut Value, id: &str) -> &'a mut Value {
    pack["rules"].as_array_mut().unwrap().iter_mut().find(|r| r["id"] == id).unwrap()
}

fn headings(report: &str) -> Vec<String> {
    report.lines().filter(|l| l.starts_with("## ")).map(str::to_string).collect()
}

#[test]
fn the_report_has_the_same_seven_sections_in_the_same_order_in_both_languages() {
    let zh = pack_report(&example(), Lang::Zh);
    let en = pack_report(&example(), Lang::En);
    let zh_h = headings(&zh);
    assert_eq!(zh_h.len(), 7, "{zh_h:?}");
    for (i, want) in ["不可修改项", "必需外设", "可配置项", "AI 可编辑区域", "高速 / 机械 / 布线约束", "冲突与缺口", "来源列表"].iter().enumerate() {
        assert!(zh_h[i].contains(want), "{} should contain {want}", zh_h[i]);
        assert!(zh_h[i].starts_with(&format!("## {}.", i + 1)));
    }
    let en_h = headings(&en);
    assert_eq!(en_h.len(), 7);
    assert!(en_h[0].contains("must not be changed") && en_h[6].contains("Sources"));
    // the four questions are on the first page
    assert!(zh.contains("什么不能动、什么必须保留、什么可以改、依据是什么"));
    // deterministic
    assert_eq!(zh, pack_report(&example(), Lang::Zh));
}

#[test]
fn an_explicit_rule_is_reported_with_document_version_page_and_the_words_of_the_manual() {
    let zh = pack_report(&example(), Lang::Zh);
    let item: Vec<&str> = zh.lines().skip_while(|l| !l.contains("**C-RESERVED**")).take(2).collect();
    assert!(item[0].contains("不得使用 FM-100 10, 11"), "{}", item[0]);
    let ev = item[1];
    for needle in ["SRC-FM100-DS", "Acme FM-100 datasheet (fictional) v1.0", "官方手册", "第 5 页", "Pin definitions", "RSVD1 and RSVD2 are connected to the flash inside the module; do not use.", "原文明确说明"] {
        assert!(ev.contains(needle), "{needle} is missing from {ev}");
    }
}

#[test]
fn a_conditional_rule_keeps_its_condition() {
    let zh = pack_report(&example(), Lang::Zh);
    let at = zh.find("**C-USB**").unwrap();
    let block = &zh[at..at + 600];
    assert!(block.contains("适用条件: the USB interface is used"), "{block}");
    assert!(block.contains("阻抗 90 Ω") && block.contains("偏斜 0.5 mm"), "{block}");
    // the unknown part of the same interface is not hidden behind it
    assert!(zh.contains("**C-USB-GAP** 资料未说明"), "the gap is in section 6");
}

#[test]
fn a_reference_board_that_differs_from_the_manual_is_a_conflict_not_a_pick() {
    // the manual says the default level of IO0 is high; the open board's schematic pulls it low
    let mut p = example();
    let manual = rule_mut(&mut p, "C-STRAP-IO0")["evidence"].clone();
    let board = json!([{"source_id": "SRC-OPENBOARD", "locator": {"section": "Boot"}, "quote": "R9 pulls IO0 low", "basis": "observed"}]);
    let r = rule_mut(&mut p, "C-STRAP-IO0");
    r["modifiability"] = json!("conflict");
    r.as_object_mut().unwrap().remove("evidence");
    r["params"] = json!({"sampled_at": "reset", "default_level": null, "external_drive_allowed": null});
    r["conflict"] = json!({"candidates": [{"value": "high", "evidence": manual}, {"value": "low", "evidence": board}], "note": "manual v1.0 against the open board"});
    assert!(hcp::check(&p).is_empty(), "{:#?}", hcp::check(&p));
    let zh = pack_report(&p, Lang::Zh);
    let sec6 = &zh[zh.find("## 6.").unwrap()..zh.find("## 7.").unwrap()];
    assert!(sec6.contains("C-STRAP-IO0") && sec6.contains("资料冲突") && sec6.contains("需要人工裁决"), "{sec6}");
    assert!(sec6.contains("说法 1: `\"high\"`") && sec6.contains("说法 2: `\"low\"`"), "{sec6}");
    assert!(sec6.contains("原文明确说明") && sec6.contains("参考板观察"), "both sides are shown, labelled differently: {sec6}");
    // ... and it does not appear among the fixed rules
    let sec1 = &zh[zh.find("## 1.").unwrap()..zh.find("## 2.").unwrap()];
    assert!(!sec1.contains("C-STRAP-IO0"));
    // a conflict never runs: the compiler marks it and the validator reports it
    let ir = compile::compile(&p).unwrap();
    assert_eq!(ir.rule("C-STRAP-IO0").unwrap().status, "conflict");
    let v = validate(&ir, &legal(), "demo", None).unwrap();
    assert!(v.findings.iter().any(|f| f.rule_id == "C-STRAP-IO0" && f.verdict == Verdict::Conflict));
    assert!(!v.complete);
    // the lazy way out is refused: the board's behaviour as the chip's requirement
    let mut lazy = example();
    rule_mut(&mut lazy, "C-STRAP-IO0")["evidence"] = board;
    assert!(hcp::check(&lazy).iter().any(|d| d.code == "weak_evidence"));
}

#[test]
fn missing_data_stays_unknown_and_is_listed_as_a_gap() {
    let zh = pack_report(&example(), Lang::Zh);
    let sec6 = &zh[zh.find("## 6.").unwrap()..zh.find("## 7.").unwrap()];
    assert!(sec6.contains("**C-USB-GAP**") && sec6.contains("The documents do not give a spacing"), "{sec6}");
    assert!(sec6.contains("间距 未知"), "the empty slot is shown as unknown, not as a number: {sec6}");
    // guessing the number is refused, and an unknown rule has no executable form
    let mut guessed = example();
    rule_mut(&mut guessed, "C-USB-GAP")["params"]["gap_mm"] = json!(0.15);
    assert!(hcp::check(&guessed).iter().any(|d| d.code == "unknown_value"));
    assert_eq!(compile::compile(&example()).unwrap().rule("C-USB-GAP").unwrap().status, "unknown");
}

#[test]
fn derivations_observations_and_pending_notes_are_never_labelled_as_the_manual() {
    let mut p = example();
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-DERIVED", "kind": "reserved_pin", "modifiability": "derived", "subject": {"device": "DEV-FM100"},
        "params": {"pins": ["11"]}, "derived_from": ["C-RESERVED"],
        "evidence": [{"source_id": "SRC-FM100-DS", "locator": {"page": 5}, "quote": "RSVD1 and RSVD2 are connected to the flash", "basis": "derived"}]
    }));
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-OBS", "kind": "clearance", "modifiability": "configurable", "params": {"clearance_mm": 0.2},
        "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Rules"}, "quote": "0.2 mm clearance", "basis": "observed"}]
    }));
    p["rules"].as_array_mut().unwrap().push(json!({
        "id": "C-PEND", "kind": "trace_width", "modifiability": "configurable", "params": {"width_mm": 0.3},
        "evidence": [{"source_id": "SRC-OPENBOARD", "locator": {"section": "Rules"}, "quote": "maybe 0.3 mm", "basis": "pending"}]
    }));
    let d = hcp::check(&p);
    assert!(d.iter().all(|x| x.severity == fr_constraint::diag::Severity::Warning) && d.iter().any(|x| x.code == "pending_evidence"), "{d:#?}");
    let zh = pack_report(&p, Lang::Zh);
    let item = |id: &str| zh.lines().skip_while(|l| !l.contains(&format!("**{id}**"))).take(4).collect::<Vec<_>>().join("\n");
    assert!(item("C-DERIVED").contains("由多条规则推导") && item("C-DERIVED").contains("这是推导，不是手册原文"));
    assert!(item("C-OBS").contains("参考板观察") && !item("C-OBS").contains("原文明确说明"));
    assert!(item("C-PEND").contains("待确认"));
    let sec6 = &zh[zh.find("## 6.").unwrap()..zh.find("## 7.").unwrap()];
    assert!(sec6.contains("待确认的依据"), "pending evidence is a gap: {sec6}");
}

#[test]
fn a_mistaken_recall_by_a_retrieval_tool_cannot_get_into_a_pack() {
    // retrieval data in a pack
    let mut p = example();
    rule_mut(&mut p, "C-GND")["evidence"][0]["x_score"] = json!(0.91);
    p["x_vector"] = json!([0.1]);
    let d = hcp::check(&p);
    assert!(d.iter().filter(|x| x.code == "vector_field").count() >= 2, "{d:#?}");
    assert!(compile::compile(&p).is_err());
    // a recalled passage of another version/model: the pack cites a document of a similar model
    let mut q = example();
    q["devices"].as_array_mut().unwrap().push(json!({"id": "DEV-FM100U", "vendor": "Acme", "part_number": "FM-100U", "package": "module", "pins": [{"number": "1", "name": "GND", "class": "ground"}]}));
    q["sources"].as_array_mut().unwrap().push(json!({
        "source_id": "SRC-FM100U-DS", "document_id": "DOC-FM100U-DS", "title": "FM-100U datasheet", "version": "v1.0", "level": "official_manual",
        "retrieved": "2026-11-16", "sha256": "1".repeat(64), "applies_to": ["DEV-FM100U"]
    }));
    // the FM-100's rule quotes the FM-100U's manual
    rule_mut(&mut q, "C-RESERVED")["evidence"] = json!([{"source_id": "SRC-FM100U-DS", "locator": {"page": 5}, "quote": "RSVD pins", "basis": "explicit"}]);
    let d = hcp::check(&q);
    let e = d.iter().find(|x| x.code == "source_not_applicable").expect("refused");
    assert_eq!(e.object.as_deref(), Some("C-RESERVED"));
}

#[test]
fn the_ai_editable_whitelist_is_in_the_report_and_everything_else_is_read_only() {
    let zh = pack_report(&example(), Lang::Zh);
    let sec4 = &zh[zh.find("## 4.").unwrap()..zh.find("## 5.").unwrap()];
    for needle in ["默认所有原理图、PCB、引脚和机械对象只读", "REG-USER", "BLK-LED", "可用引脚池: DEV-FM100 5", "add_component, connect, add_track", "不可变: 元件: U1, J1"] {
        assert!(sec4.contains(needle), "{needle} is missing from {sec4}");
    }
    let mut p = example();
    p.as_object_mut().unwrap().remove("base_design");
    p["regions"].as_array_mut().unwrap().retain(|r| r["id"] != "REG-USER");
    p["peripheral_blocks"].as_array_mut().unwrap().retain(|b| b["id"] != "BLK-LED");
    let zh = pack_report(&p, Lang::Zh);
    assert!(zh.contains("没有 base_design：没有任何东西可由 AI 修改"));
}

#[test]
fn quotations_are_kept_as_written_and_the_sources_table_is_complete() {
    let mut p = example();
    rule_mut(&mut p, "C-GND")["evidence"][0]["quote"] = json!("GND：接地。 (原文为中文)");
    let zh = pack_report(&p, Lang::Zh);
    assert!(zh.contains("\"GND：接地。 (原文为中文)\""));
    let sec7 = &zh[zh.find("## 7.").unwrap()..];
    for id in ["SRC-FM100-DS / DOC-FM100-DS", "SRC-FM100-HDG / DOC-FM100-HDG", "SRC-OPENBOARD / DOC-OPENBOARD"] {
        assert!(sec7.contains(id), "{id}");
    }
    assert!(sec7.contains("官方手册") && sec7.contains("开源参考板") && sec7.contains("hcp-manager evidence verify"));
}
