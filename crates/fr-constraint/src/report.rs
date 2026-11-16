//! Human-readable reports (Markdown) in Chinese or English: of a validation, and (see `pack_report`)
//! of a pack. The words around the data are translated; quotations stay as they were written.

use crate::ir::IrEvidence;
use crate::validate::{Finding, Validation, Verdict};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Zh,
    En,
}

impl Lang {
    pub fn parse(s: &str) -> Option<Lang> {
        match s {
            "zh" | "zh-CN" | "zh_CN" => Some(Lang::Zh),
            "en" | "en-US" => Some(Lang::En),
            _ => None,
        }
    }
}

pub fn t(lang: Lang, zh: &'static str, en: &'static str) -> &'static str {
    match lang {
        Lang::Zh => zh,
        Lang::En => en,
    }
}

pub fn evidence_line(e: &IrEvidence) -> String {
    let loc = &e.locator;
    let mut parts: Vec<String> = Vec::new();
    if let Some(p) = loc.get("page").and_then(|v| v.as_u64()) {
        parts.push(format!("p.{p}"));
    }
    for k in ["section", "table", "figure"] {
        if let Some(v) = loc.get(k).and_then(|v| v.as_str()) {
            parts.push(v.to_string());
        }
    }
    let chunk = e.chunk_id.as_ref().map(|c| format!(" [{c}]")).unwrap_or_default();
    format!("{} {}{} ({}): \"{}\"", e.source_id, parts.join(", "), chunk, e.basis, e.quote.replace('\n', " "))
}

fn verdict_label(lang: Lang, v: Verdict) -> &'static str {
    match v {
        Verdict::Fail => t(lang, "失败", "failed"),
        Verdict::Unchecked => t(lang, "未检查", "not checked"),
        Verdict::Unsupported => t(lang, "不支持", "unsupported"),
        Verdict::Conflict => t(lang, "资料冲突", "conflict"),
        Verdict::Unknown => t(lang, "资料未说明", "unknown"),
        Verdict::NotApplicable => t(lang, "不适用", "not applicable"),
        Verdict::Pass => t(lang, "通过", "passed"),
    }
}

fn finding_block(lang: Lang, f: &Finding, out: &mut String) {
    out.push_str(&format!("### {} `{}`{}\n\n", f.rule_id, f.kind, f.object.as_ref().map(|o| format!(" — {o}")).unwrap_or_default()));
    out.push_str(&format!("- {}: {} ({})\n", t(lang, "说明", "What"), f.message, f.severity));
    if let Some(a) = &f.actual {
        out.push_str(&format!("- {}: `{a}`\n", t(lang, "实际值", "Actual")));
    }
    if let Some(r) = &f.required {
        out.push_str(&format!("- {}: `{r}`\n", t(lang, "要求值", "Required")));
    }
    for e in &f.evidence {
        out.push_str(&format!("- {}: {}\n", t(lang, "依据", "Evidence"), evidence_line(e)));
    }
    out.push('\n');
}

/// The validation as a Markdown report.
pub fn validation_text(v: &Validation, lang: Lang) -> String {
    let mut o = String::new();
    o.push_str(&format!("# {}\n\n", t(lang, "硬件约束校验报告", "Hardware constraint validation report")));
    o.push_str(&format!(
        "- {}: **{}**{}\n",
        t(lang, "结果", "Result"),
        if v.result == "pass" { t(lang, "通过", "pass") } else { t(lang, "失败", "FAIL") },
        if v.complete { "" } else { t(lang, "（不完整：有未检查、不支持、未知或冲突的规则）", " (incomplete: some rules are unchecked, unsupported, unknown or in conflict)") }
    ));
    o.push_str(&format!("- {}: {} {} (pack sha256 `{}`)\n", t(lang, "知识包", "Pack"), v.ir["pack_id"].as_str().unwrap_or(""), v.ir["pack_version"].as_str().unwrap_or(""), v.ir["pack_sha256"].as_str().unwrap_or("")));
    o.push_str(&format!("- Constraint IR sha256 `{}`\n", v.ir["ir_sha256"].as_str().unwrap_or("")));
    o.push_str(&format!("- {}: {} (sha256 `{}`)\n\n", t(lang, "设计", "Design"), v.design["name"].as_str().unwrap_or(""), v.design["sha256"].as_str().unwrap_or("")));
    let s = &v.summary;
    o.push_str(&format!("## {}\n\n", t(lang, "摘要", "Summary")));
    o.push_str(&format!("| {} | {} |\n|---|---|\n", t(lang, "状态", "Status"), t(lang, "规则数", "Rules")));
    for (label, n) in [
        (verdict_label(lang, Verdict::Pass), s.passed),
        (verdict_label(lang, Verdict::Fail), s.failed),
        (verdict_label(lang, Verdict::NotApplicable), s.not_applicable),
        (verdict_label(lang, Verdict::Unchecked), s.unchecked),
        (verdict_label(lang, Verdict::Unsupported), s.unsupported),
        (verdict_label(lang, Verdict::Unknown), s.unknown),
        (verdict_label(lang, Verdict::Conflict), s.conflict),
    ] {
        o.push_str(&format!("| {label} | {n} |\n"));
    }
    o.push('\n');
    for verdict in [Verdict::Fail, Verdict::Unchecked, Verdict::Unsupported, Verdict::Conflict, Verdict::Unknown, Verdict::NotApplicable] {
        let list: Vec<&Finding> = v.findings.iter().filter(|f| f.verdict == verdict).collect();
        if list.is_empty() {
            continue;
        }
        o.push_str(&format!("## {} ({})\n\n", verdict_label(lang, verdict), list.len()));
        for f in list {
            finding_block(lang, f, &mut o);
        }
    }
    let passed: Vec<&Finding> = v.findings.iter().filter(|f| f.verdict == Verdict::Pass).collect();
    if !passed.is_empty() {
        o.push_str(&format!("## {} ({})\n\n", verdict_label(lang, Verdict::Pass), passed.len()));
        for f in passed {
            o.push_str(&format!("- {} `{}`\n", f.rule_id, f.kind));
        }
        o.push('\n');
    }
    o
}
