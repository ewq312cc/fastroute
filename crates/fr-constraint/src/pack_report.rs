//! The report of a pack: "what must not be touched, what must stay, what may change, and why".
//! The sections are always the same and in the same order, so that a reader (and a test) knows where
//! to look. What a rule means is said in words; quotations stay in the language of the document;
//! derivations, reference-board observations and pending notes are labelled, never passed off as
//! the manual's text.

use crate::report::{t, Lang};
use serde_json::Value;
use std::collections::BTreeMap;

fn st<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or("")
}

fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

fn level_label(lang: Lang, level: &str) -> &'static str {
    match level {
        "official_manual" => t(lang, "官方手册", "official manual"),
        "official_errata" => t(lang, "官方勘误", "official errata"),
        "official_guide" => t(lang, "官方设计指南", "official design guide"),
        "official_reference_design" => t(lang, "官方参考设计", "official reference design"),
        "official_devkit" => t(lang, "官方开发板", "official dev board"),
        "open_board" => t(lang, "开源参考板", "open board"),
        _ => t(lang, "社区资料", "community"),
    }
}

fn basis_label(lang: Lang, basis: &str) -> &'static str {
    match basis {
        "explicit" => t(lang, "原文明确说明", "stated explicitly in the document"),
        "derived" => t(lang, "由多条规则推导", "derived from several statements"),
        "observed" => t(lang, "参考板观察", "observed on a reference board"),
        _ => t(lang, "待确认", "pending confirmation"),
    }
}

fn unit_label(suffix: &str) -> &'static str {
    match suffix {
        "mm" => "mm",
        "um" => "µm",
        "mil" => "mil",
        "ohm" => "Ω",
        "kohm" => "kΩ",
        "v" => "V",
        "mv" => "mV",
        "a" => "A",
        "ma" => "mA",
        "ua" => "µA",
        "uf" => "µF",
        "nf" => "nF",
        "pf" => "pF",
        "uh" => "µH",
        "nh" => "nH",
        "hz" => "Hz",
        "khz" => "kHz",
        "mhz" => "MHz",
        "ghz" => "GHz",
        "ns" => "ns",
        "ps" => "ps",
        "us" => "µs",
        "ms" => "ms",
        "s" => "s",
        "deg" => "°",
        "pct" => "%",
        "db" => "dB",
        _ => "",
    }
}

fn param_name(lang: Lang, base: &str) -> String {
    let zh = match base {
        "impedance" => "阻抗",
        "gap" => "间距",
        "skew" => "偏斜",
        "width" => "线宽",
        "clearance" => "间距",
        "max_length" => "最大长度",
        "length_match_tolerance" => "等长容差",
        "min_width" => "最小线宽",
        "min" => "最小值",
        "max" => "最大值",
        "typ" => "典型值",
        "min_count" => "最少数量",
        "min_value" => "最小值",
        "max_value" => "最大值",
        "value" => "数值",
        _ => "",
    };
    if lang == Lang::Zh && !zh.is_empty() {
        zh.to_string()
    } else {
        base.replace('_', " ")
    }
}

/// `impedance_ohm: 90` -> "impedance 90 Ω"; `null` -> "unknown".
fn fmt_param(lang: Lang, key: &str, v: &Value) -> String {
    let (base, unit) = match key.rsplit_once('_') {
        Some((b, u)) if !unit_label(u).is_empty() || u == "count" || u == "ratio" => (b, if u == "count" { "" } else { unit_label(u) }),
        _ => (key, ""),
    };
    let shown = match v {
        Value::Null => t(lang, "未知", "unknown").to_string(),
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(", "),
        other => other.to_string(),
    };
    format!("{} {}{}", param_name(lang, base), shown, if unit.is_empty() || v.is_null() { String::new() } else { format!(" {unit}") })
}

fn join(v: &Value) -> String {
    v.as_array().map(|a| a.iter().map(|x| x.as_str().map(str::to_string).unwrap_or_else(|| x.to_string())).collect::<Vec<_>>().join(", ")).unwrap_or_default()
}

/// The words that say what a rule means (not its evidence).
fn rule_sentence(lang: Lang, r: &Value, pack: &Value) -> String {
    let p = r.get("params").cloned().unwrap_or(Value::Null);
    let subj = r.get("subject").cloned().unwrap_or(Value::Null);
    let dev = {
        let id = st(&subj, "device");
        arr(pack, "devices").iter().find(|d| st(d, "id") == id).map(|d| st(d, "part_number").to_string()).unwrap_or_else(|| id.to_string())
    };
    let pin = st(&subj, "pin").to_string();
    let block_title = |id: &str| arr(pack, "peripheral_blocks").iter().find(|b| st(b, "id") == id).map(|b| st(b, "title").to_string()).unwrap_or_else(|| id.to_string());
    let region_title = |id: &str| {
        arr(pack, "regions")
            .iter()
            .find(|b| st(b, "id") == id)
            .map(|b| if st(b, "title").is_empty() { id.to_string() } else { format!("{id} {}", st(b, "title")) })
            .unwrap_or_else(|| id.to_string())
    };
    let extra: Vec<String> = p
        .as_object()
        .map(|m| m.iter().filter(|(k, v)| (v.is_number() || v.is_null()) && !["pins", "pin", "nets", "functions"].contains(&k.as_str())).map(|(k, v)| fmt_param(lang, k, v)).collect())
        .unwrap_or_default();
    let extra_paren = if extra.is_empty() { String::new() } else { format!(" ({})", extra.join("; ")) };
    match st(r, "kind") {
        "reserved_pin" => format!("{} {} {}{}{}", t(lang, "不得使用", "Do not use pins"), dev, join(&p["pins"]), p["reason"].as_str().map(|x| format!(": {x}")).unwrap_or_default(), extra_paren),
        "nc_pin" => format!("{} {} {} {}", dev, t(lang, "的引脚", "pins"), join(&p["pins"]), t(lang, "必须悬空（NC）", "must be left unconnected (NC)")),
        "fixed_net" => format!("{} {} {} {} {}", dev, t(lang, "的引脚", "pin"), p["pin"].as_str().unwrap_or(""), t(lang, "必须接到网络", "must be on a net matching"), p["net_pattern"].as_str().unwrap_or("")),
        "exclusive_functions" => format!("{} {} {}", dev, t(lang, "的这些功能不能同时使用:", "functions that cannot be in use together:"), join(&p["functions"])),
        "required_block" => format!("{} {}", t(lang, "必须有外设块", "The design must contain the block"), block_title(st(&subj, "block"))),
        "strapping" => format!(
            "{} {} {}: {} {}, {} {}, {}",
            dev,
            t(lang, "的启动配置脚", "strapping pin"),
            pin,
            t(lang, "采样时刻", "sampled at"),
            p["sampled_at"].as_str().unwrap_or(""),
            t(lang, "默认电平", "default level"),
            p["default_level"].as_str().unwrap_or(""),
            if p["external_drive_allowed"].as_bool() == Some(true) { t(lang, "允许外部驱动", "external drive allowed") } else { t(lang, "不允许外部强拉/驱动", "no external drive") }
        ),
        "reset_pin" => format!("{} {} {}: {} {}", dev, t(lang, "的复位脚", "reset pin"), pin, t(lang, "有效电平", "active level"), p["active_level"].as_str().unwrap_or("")),
        "boot_pin" => format!("{} {} {}: {} {}{}", dev, t(lang, "的启动模式脚", "boot-mode pin"), pin, t(lang, "电平", "level"), p["boot_level"].as_str().unwrap_or(""), p["mode"].as_str().map(|m| format!(" → {m}")).unwrap_or_default()),
        "electrical" => format!("{}{}: {}", p["quantity"].as_str().unwrap_or(""), p["condition"].as_str().map(|c| format!(" ({c})")).unwrap_or_default(), extra.join("; ")),
        "high_speed" => format!(
            "{} {} ({}){}",
            t(lang, "接口网络", "Interface nets"),
            join(&p["nets"]),
            if p["differential"].as_bool() == Some(true) { t(lang, "差分", "differential") } else { t(lang, "单端", "single-ended") },
            extra_paren
        ),
        "clearance" => format!("{} {}", t(lang, "最小间距", "Minimum clearance"), fmt_param(lang, "clearance_mm", &p["clearance_mm"])),
        "trace_width" => format!("{} {}{}", t(lang, "线宽不小于", "Trace width at least"), fmt_param(lang, "width_mm", &p["width_mm"]), if p["nets"].is_array() { format!(" ({})", join(&p["nets"])) } else { String::new() }),
        "keepout" => format!("{} {} {} {}", t(lang, "区域", "Region"), region_title(st(&subj, "region")), t(lang, "内禁止:", "forbids:"), join(&p["forbid"])),
        "mechanical_fixed" => format!("{} {}", region_title(st(&subj, "region")), t(lang, "的位置固定，不得移动", "stays where it is")),
        other => other.to_string(),
    }
}

/// `SRC-DS: <title> <version> [level], p.8, §3.3 — "quote" (basis)`
fn evidence_text(lang: Lang, pack: &Value, e: &Value) -> String {
    let sid = st(e, "source_id");
    let src = arr(pack, "sources").iter().find(|s| st(s, "source_id") == sid);
    let doc = src.map(|s| format!("{} {} [{}]", st(s, "title"), st(s, "version"), level_label(lang, st(s, "level")))).unwrap_or_default();
    let loc = &e["locator"];
    let mut at: Vec<String> = Vec::new();
    if let Some(p) = loc.get("page").and_then(Value::as_u64) {
        at.push(format!("{}{p}{}", t(lang, "第 ", "p."), t(lang, " 页", "")));
    }
    for k in ["section", "table", "figure"] {
        if let Some(v) = loc.get(k).and_then(Value::as_str) {
            at.push(v.to_string());
        }
    }
    let chunk = e.get("chunk_id").and_then(Value::as_str).map(|c| format!(" `{c}`")).unwrap_or_default();
    format!("{sid}: {doc}, {}{chunk} — \"{}\" ({})", at.join(", "), st(e, "quote").replace('\n', " "), basis_label(lang, st(e, "basis")))
}

fn evidence_block(lang: Lang, pack: &Value, ev: &[Value], out: &mut String) {
    for e in ev {
        out.push_str(&format!("  - {}: {}\n", t(lang, "依据", "Evidence"), evidence_text(lang, pack, e)));
    }
}

fn rule_item(lang: Lang, pack: &Value, r: &Value, out: &mut String) {
    out.push_str(&format!("- **{}** {}— {}\n", st(r, "id"), if st(r, "title").is_empty() { String::new() } else { format!("{} ", st(r, "title")) }, rule_sentence(lang, r, pack)));
    let when: Vec<&str> = arr(r, "applies_when").iter().filter_map(Value::as_str).collect();
    if !when.is_empty() {
        out.push_str(&format!("  - {}: {}\n", t(lang, "适用条件", "Applies when"), when.join("; ")));
    }
    if st(r, "modifiability") == "derived" {
        out.push_str(&format!("  - {}: {} ({})\n", t(lang, "推导自", "Derived from"), join(&r["derived_from"]), t(lang, "这是推导，不是手册原文", "a derivation, not a quotation")));
    }
    if st(r, "severity") == "warning" {
        out.push_str(&format!("  - {}\n", t(lang, "严重级别: 警告（不使校验失败）", "Severity: warning (does not fail a validation)")));
    }
    evidence_block(lang, pack, arr(r, "evidence"), out);
}

fn region_sentence(lang: Lang, g: &Value) -> String {
    let kind = match st(g, "kind") {
        "board_outline" => t(lang, "板框", "board outline"),
        "mount_hole" => t(lang, "安装孔", "mounting hole"),
        "connector" => t(lang, "连接器", "connector"),
        "module_body" => t(lang, "模组本体", "module body"),
        "antenna_keepout" => t(lang, "天线禁区", "antenna keep-out"),
        "keepout" => t(lang, "禁区", "keep-out"),
        _ => t(lang, "可编辑区域", "editable region"),
    };
    let shape = if let Some(poly) = g.get("polygon").and_then(Value::as_array) {
        let pts: Vec<(f64, f64)> = poly.iter().filter_map(|p| Some((p.get(0)?.as_f64()?, p.get(1)?.as_f64()?))).collect();
        let (minx, maxx) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
        let (miny, maxy) = pts.iter().fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
        format!("x {minx}..{maxx} mm, y {miny}..{maxy} mm ({} {})", pts.len(), t(lang, "个顶点", "vertices"))
    } else if let Some(c) = g.get("circle") {
        format!("{} {} mm, r {} mm", t(lang, "圆心", "center"), c["center"], c["radius_mm"])
    } else {
        String::new()
    };
    let layers = join(&g["layers"]);
    format!("{}{kind}, {shape}{}", if st(g, "title").is_empty() { String::new() } else { format!("{} — ", st(g, "title")) }, if layers.is_empty() { String::new() } else { format!(", {}: {layers}", t(lang, "层", "layers")) })
}

fn block_item(lang: Lang, pack: &Value, b: &Value, out: &mut String) {
    out.push_str(&format!("- **{}** {} ({})\n", st(b, "id"), st(b, "title"), st(b, "kind")));
    for c in arr(b, "components") {
        let mut limits: Vec<String> = Vec::new();
        for k in ["min_value_nf", "max_value_nf", "value_ohm"] {
            if let Some(v) = c.get(k) {
                limits.push(fmt_param(lang, k, v));
            }
        }
        out.push_str(&format!("  - {}: {} × {}{}\n", st(c, "role"), c.get("min_count").and_then(Value::as_u64).unwrap_or(1), st(c, "kind"), if limits.is_empty() { String::new() } else { format!(" ({})", limits.join(", ")) }));
    }
    for c in arr(b, "connects") {
        out.push_str(&format!("  - {} {} → {}\n", t(lang, "连接", "connects"), st(c, "pin"), st(c, "net_pattern")));
    }
    if !st(b, "missing").is_empty() {
        out.push_str(&format!("  - {}: {}\n", t(lang, "缺口", "Missing"), st(b, "missing")));
    }
    evidence_block(lang, pack, arr(b, "evidence"), out);
}

fn none(lang: Lang, out: &mut String) {
    out.push_str(&format!("{}\n", t(lang, "（没有）", "(none)")));
}

fn collect_pending(v: &Value, path: &str, out: &mut Vec<String>) {
    match v {
        Value::Object(m) => {
            if m.get("basis").and_then(Value::as_str) == Some("pending") {
                out.push(path.to_string());
            }
            for (k, c) in m {
                collect_pending(c, &format!("{path}.{k}"), out);
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                collect_pending(c, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// The pack as a readable Markdown report. Sections, always in this order: 1 cannot be changed,
/// 2 must stay, 3 may be configured, 4 where an AI may edit, 5 high-speed / mechanical / routing,
/// 6 conflicts and gaps, 7 sources.
pub fn pack_report(pack: &Value, lang: Lang) -> String {
    let mut o = String::new();
    let review = pack["pack"]["review"]["status"].as_str().unwrap_or("draft");
    o.push_str(&format!("# {} — {}\n\n", st(&pack["pack"], "title"), t(lang, "硬件约束知识包报告", "Hardware constraint pack report")));
    o.push_str(&format!(
        "> {}\n\n",
        t(
            lang,
            "这份报告回答四个问题：**什么不能动、什么必须保留、什么可以改、依据是什么**。每一条都列出来源文档、版本、页码/章节和原文摘录；“推导”“参考板观察”“待确认”不会被写成手册原文。",
            "This report answers four questions: **what must not be changed, what must stay, what may change, and what the evidence is**. Every item lists document, version, page/section and the quoted words; derivations, reference-board observations and pending notes are never presented as the manual's text."
        )
    ));
    o.push_str(&format!("- {}: `{}` {} · {}: {}\n", t(lang, "知识包", "Pack"), st(&pack["pack"], "id"), st(&pack["pack"], "version"), t(lang, "抽取者", "Extracted by"), st(&pack["pack"]["extractor"], "kind")));
    o.push_str(&format!(
        "- {}: **{}**{}\n",
        t(lang, "审查状态", "Review"),
        review,
        if review == "draft" { t(lang, "（草稿：尚未经人工审查，不得作为最终依据）", " (draft: not reviewed by a person; not a final basis)") } else { "" }
    ));
    for d in arr(pack, "devices") {
        o.push_str(&format!("- {}: {} {} {} ({}), {} {}\n", t(lang, "器件", "Device"), st(d, "vendor"), st(d, "part_number"), st(d, "revision"), st(d, "id"), arr(d, "pins").len(), t(lang, "个引脚", "pins")));
    }
    o.push('\n');

    let rules = arr(pack, "rules");
    let blocks = arr(pack, "peripheral_blocks");
    let special = |k: &str| ["high_speed", "keepout", "mechanical_fixed", "clearance", "trace_width"].contains(&k);
    let is_edit = |r: &Value| st(r, "modifiability") == "ai_editable";
    let main_rule = |r: &&Value| !special(st(r, "kind")) && !is_edit(r);

    // 1. fixed
    o.push_str(&format!("## 1. {}\n\n", t(lang, "不可修改项（固定）", "What must not be changed (fixed)")));
    let fixed: Vec<&Value> = rules.iter().filter(main_rule).filter(|r| st(r, "modifiability") == "fixed").collect();
    let fixed_blocks: Vec<&Value> = blocks.iter().filter(|b| st(b, "modifiability") == "fixed").collect();
    for r in &fixed {
        rule_item(lang, pack, r, &mut o);
    }
    for b in &fixed_blocks {
        block_item(lang, pack, b, &mut o);
    }
    if fixed.is_empty() && fixed_blocks.is_empty() {
        none(lang, &mut o);
    }
    o.push('\n');

    // 2. required
    o.push_str(&format!("## 2. {}\n\n", t(lang, "必需外设与必须保留项", "What must stay (required peripherals and rules)")));
    let blocks_required: Vec<&Value> = blocks.iter().filter(|b| st(b, "modifiability") == "required").collect();
    let required: Vec<&Value> = rules.iter().filter(main_rule).filter(|r| st(r, "modifiability") == "required" && st(r, "kind") != "required_block").collect();
    for b in &blocks_required {
        block_item(lang, pack, b, &mut o);
    }
    for r in &required {
        rule_item(lang, pack, r, &mut o);
    }
    if blocks_required.is_empty() && required.is_empty() {
        none(lang, &mut o);
    }
    o.push('\n');

    // 3. configurable, optional, derived
    o.push_str(&format!("## 3. {}\n\n", t(lang, "可配置项、可选项与推导项", "What may be configured (configurable, optional, derived)")));
    let mut any = false;
    for r in rules.iter().filter(main_rule).filter(|r| ["configurable", "optional", "derived"].contains(&st(r, "modifiability"))) {
        let tag = match st(r, "modifiability") {
            "configurable" => t(lang, "可配置", "configurable"),
            "optional" => t(lang, "可选", "optional"),
            _ => t(lang, "推导", "derived"),
        };
        o.push_str(&format!("[{tag}]\n"));
        rule_item(lang, pack, r, &mut o);
        any = true;
    }
    for b in blocks.iter().filter(|b| ["configurable", "optional"].contains(&st(b, "modifiability"))) {
        block_item(lang, pack, b, &mut o);
        any = true;
    }
    if !any {
        none(lang, &mut o);
    }
    o.push('\n');

    // 4. AI editable
    o.push_str(&format!("## 4. {}\n\n", t(lang, "AI 可编辑区域（白名单）", "Where an AI may edit (the whitelist)")));
    o.push_str(&format!("{}\n\n", t(lang, "默认所有原理图、PCB、引脚和机械对象只读；只有下面列出的内容可以被 AI 修改，其余一律不得改动。", "Everything is read-only by default; only what is listed here may be changed by an AI.")));
    if let Some(base) = pack.get("base_design") {
        o.push_str(&format!("- {}: {} {}\n", t(lang, "基板", "Base design"), st(base, "id"), st(base, "title")));
        if let Some(repo) = base["binding"].get("repo") {
            o.push_str(&format!("  - {}: {} @ `{}`\n", t(lang, "绑定仓库", "bound to"), st(repo, "url"), st(repo, "commit")));
        }
        for f in arr(&base["binding"], "files") {
            o.push_str(&format!("  - {}: {} `{}`\n", t(lang, "绑定文件", "bound file"), st(f, "path"), st(f, "sha256")));
        }
        let im = &base["immutable"];
        let mut fixed_things: Vec<String> = Vec::new();
        for (k, zh, en) in [("components", "元件", "components"), ("pins", "引脚", "pins"), ("connectors", "连接器", "connectors"), ("regions", "区域", "regions")] {
            if !arr(im, k).is_empty() {
                fixed_things.push(format!("{}: {}", t(lang, zh, en), join(&im[k])));
            }
        }
        if im["board_outline"].as_bool() == Some(true) {
            fixed_things.push(t(lang, "板框", "board outline").to_string());
        }
        if im["mount_holes"].as_bool() == Some(true) {
            fixed_things.push(t(lang, "安装孔", "mounting holes").to_string());
        }
        if !fixed_things.is_empty() {
            o.push_str(&format!("  - {}: {}\n", t(lang, "不可变", "immutable"), fixed_things.join("; ")));
        }
        for e in arr(base, "editable_regions") {
            let g = arr(pack, "regions").iter().find(|g| st(g, "id") == st(e, "region"));
            o.push_str(&format!("- **{}** {}\n", st(e, "region"), g.map(|g| region_sentence(lang, g)).unwrap_or_default()));
            if !arr(e, "allowed_blocks").is_empty() {
                o.push_str(&format!("  - {}: {}\n", t(lang, "允许添加的外设块", "blocks that may be added"), join(&e["allowed_blocks"])));
            }
            for pool in arr(e, "pin_pool") {
                o.push_str(&format!("  - {}: {} {}\n", t(lang, "可用引脚池", "pin pool"), st(pool, "device"), join(&pool["pins"])));
            }
            if !arr(e, "allowed_ops").is_empty() {
                o.push_str(&format!("  - {}: {}\n", t(lang, "允许的操作", "allowed operations"), join(&e["allowed_ops"])));
            }
            if let Some(g) = g {
                evidence_block(lang, pack, arr(g, "evidence"), &mut o);
            }
        }
    } else {
        o.push_str(&format!("{}\n", t(lang, "（没有 base_design：没有任何东西可由 AI 修改）", "(no base_design: nothing may be changed by an AI)")));
    }
    for r in rules.iter().filter(|r| is_edit(r)) {
        rule_item(lang, pack, r, &mut o);
    }
    o.push('\n');

    // 5. high speed, mechanical, routing
    o.push_str(&format!("## 5. {}\n\n", t(lang, "高速 / 机械 / 布线约束", "High-speed, mechanical and routing constraints")));
    let mut any = false;
    for r in rules.iter().filter(|r| special(st(r, "kind")) && !["unknown", "conflict"].contains(&st(r, "modifiability"))) {
        o.push_str(&format!("[{}]\n", st(r, "modifiability")));
        rule_item(lang, pack, r, &mut o);
        any = true;
    }
    let mut by_kind: BTreeMap<&str, Vec<&Value>> = BTreeMap::new();
    for g in arr(pack, "regions") {
        by_kind.entry(st(g, "kind")).or_default().push(g);
    }
    for gs in by_kind.values() {
        for g in gs {
            o.push_str(&format!("- **{}** {} [{}]\n", st(g, "id"), region_sentence(lang, g), st(g, "modifiability")));
            evidence_block(lang, pack, arr(g, "evidence"), &mut o);
            any = true;
        }
    }
    if !any {
        none(lang, &mut o);
    }
    o.push('\n');

    // 6. conflicts and gaps
    o.push_str(&format!("## 6. {}\n\n", t(lang, "冲突与缺口", "Conflicts and gaps")));
    let mut any = false;
    for r in rules.iter().filter(|r| st(r, "modifiability") == "conflict") {
        any = true;
        o.push_str(&format!("- **{}** {} {}\n", st(r, "id"), t(lang, "资料冲突:", "documents disagree:"), rule_sentence(lang, r, pack)));
        for (i, c) in arr(&r["conflict"], "candidates").iter().enumerate() {
            o.push_str(&format!("  - {} {}: `{}`\n", t(lang, "说法", "Candidate"), i + 1, c["value"]));
            evidence_block(lang, pack, arr(c, "evidence"), &mut o);
        }
        o.push_str(&format!("  - {}\n", t(lang, "需要人工裁决；在此之前不会生成可执行规则。", "A person has to decide; no executable rule is made until then.")));
    }
    for r in rules.iter().filter(|r| st(r, "modifiability") == "unknown") {
        any = true;
        o.push_str(&format!("- **{}** {} {}\n  - {}: {}\n", st(r, "id"), t(lang, "资料未说明:", "not stated in the documents:"), rule_sentence(lang, r, pack), t(lang, "缺口", "Missing"), st(r, "missing")));
    }
    for b in blocks.iter().filter(|b| st(b, "modifiability") == "unknown") {
        any = true;
        o.push_str(&format!("- **{}** {} {}: {}\n", st(b, "id"), st(b, "title"), t(lang, "资料未说明", "unknown"), st(b, "missing")));
    }
    for g in arr(pack, "regions").iter().filter(|g| st(g, "modifiability") == "unknown") {
        any = true;
        o.push_str(&format!("- **{}** {}: {}\n", st(g, "id"), t(lang, "资料未说明", "unknown"), st(g, "missing")));
    }
    let mut pending: Vec<String> = Vec::new();
    collect_pending(pack, "$", &mut pending);
    for p in &pending {
        any = true;
        o.push_str(&format!("- {} `{p}`\n", t(lang, "待确认的依据:", "evidence still pending:")));
    }
    if !any {
        o.push_str(&format!("{}\n", t(lang, "（没有冲突，也没有已知缺口）", "(no conflicts and no known gaps)")));
    }
    o.push('\n');

    // 7. sources
    o.push_str(&format!("## 7. {}\n\n", t(lang, "来源列表", "Sources")));
    o.push_str(&format!("| {} | {} | {} | {} | {} | SHA-256 |\n|---|---|---|---|---|---|\n", t(lang, "编号", "Id"), t(lang, "文档", "Document"), t(lang, "版本", "Version"), t(lang, "级别", "Level"), t(lang, "获取日期", "Retrieved")));
    for s in arr(pack, "sources") {
        let sha = st(s, "sha256");
        o.push_str(&format!(
            "| {} / {} | {}{} | {} | {} | {} | `{}` |\n",
            st(s, "source_id"),
            st(s, "document_id"),
            st(s, "title"),
            if st(s, "url").is_empty() { String::new() } else { format!(" ({})", st(s, "url")) },
            st(s, "version"),
            level_label(lang, st(s, "level")),
            st(s, "retrieved"),
            &sha[..16.min(sha.len())]
        ));
    }
    o.push_str(&format!(
        "\n{}\n",
        t(
            lang,
            "回跳原文：`hcp-manager evidence verify <pack>` 检查每条引用的文档、版本、摘要和页码上的原文；`hcp-manager search \"…\" --device <型号>` 找到同一版本的原文块。",
            "Back to the original text: `hcp-manager evidence verify <pack>` checks every citation (document, version, digest, quotation on the page); `hcp-manager search \"...\" --device <model>` finds blocks of the same version."
        )
    ));
    o
}
