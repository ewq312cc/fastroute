//! `fastroute rules`: rule drafts from people, scripts or AI models - checked, confirmed, applied.

mod common;
use common::*;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn examples() -> PathBuf {
    manifest_dir().join("../../docs/examples/rules")
}

/// Runs in the examples directory (so that relative paths are those of the committed files).
fn rules(args: &[&str]) -> std::process::Output {
    let mut a = vec!["rules"];
    a.extend(args);
    run_in(&examples(), &a)
}

fn check(draft: &str) -> (i32, Value) {
    let o = rules(&["check", draft, "--design", "design-inspect.json", "-q"]);
    (code(&o), serde_json::from_slice(&o.stdout).unwrap_or_else(|e| panic!("{draft}: no report ({e}): {}", stderr(&o))))
}

fn codes(report: &Value, key: &str) -> Vec<String> {
    let mut v: Vec<String> = report[key].as_array().unwrap().iter().map(|e| e["code"].as_str().unwrap().to_string()).collect();
    v.sort();
    v.dedup();
    v
}

fn rule<'a>(draft: &'a Value, id: &str) -> &'a Value {
    draft["rules"].as_array().unwrap().iter().find(|r| r["id"] == id).unwrap()
}

fn generate(text_file: &str) -> Value {
    let d = tmp("gen");
    let out = d.join("draft.json");
    let o = rules(&["draft", "--text-file", text_file, "--design", "design-inspect.json", "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{text_file}: {}", stderr(&o));
    json(&out)
}

// ---- the published pieces ----------------------------------------------------------------

#[test]
fn schema_capabilities_and_docs_agree() {
    let o = rules(&["schema"]);
    assert_eq!(code(&o), 0);
    assert_eq!(o.stdout, std::fs::read(manifest_dir().join("../../docs/schema/rule-draft-1.0.schema.json")).unwrap(), "`rules schema` prints the published file");
    let o = rules(&["schema", "--confirmation"]);
    assert_eq!(o.stdout, std::fs::read(manifest_dir().join("../../docs/schema/rule-confirmation-1.0.schema.json")).unwrap());

    let o = rules(&["capabilities"]);
    assert_eq!(code(&o), 0);
    let cap: Value = serde_json::from_slice(&o.stdout).unwrap();
    let kinds = cap["kinds"].as_array().unwrap();
    // every kind in the schema is described, with what it maps to or why not
    let schema = json(&manifest_dir().join("../../docs/schema/rule-draft-1.0.schema.json"));
    let schema_kinds: Vec<&str> = schema["$defs"]["rule"]["properties"]["kind"]["enum"].as_array().unwrap().iter().map(|k| k.as_str().unwrap()).collect();
    let mut cap_kinds: Vec<&str> = kinds.iter().map(|k| k["kind"].as_str().unwrap()).collect();
    cap_kinds.sort();
    let mut sk = schema_kinds.clone();
    sk.sort();
    assert_eq!(cap_kinds, sk);
    for k in kinds {
        assert!(!k["maps_to"].as_str().unwrap().is_empty());
        assert_eq!(k["executable"].as_bool().unwrap(), matches!(k["capability"].as_str().unwrap(), "supported" | "via_kicad"));
    }
    // the mapping table of the documentation lists every kind with its capability
    let doc = std::fs::read_to_string(manifest_dir().join("../../docs/AI_RULES.md")).unwrap();
    for k in kinds {
        let row = doc.lines().find(|l| l.starts_with(&format!("| `{}`", k["kind"].as_str().unwrap()))).unwrap_or_else(|| panic!("docs/AI_RULES.md has no table row for {}", k["kind"]));
        assert!(row.contains(&format!("`{}`", k["capability"].as_str().unwrap())), "{row}");
    }
}

#[test]
fn help_of_every_subcommand() {
    assert_eq!(code(&rules(&[])), 0);
    assert!(stdout(&rules(&[])).contains("check") && stdout(&rules(&[])).contains("apply"));
    for sub in ["schema", "capabilities", "draft", "check", "confirm-template", "apply"] {
        let o = rules(&[sub, "--help"]);
        assert_eq!(code(&o), 0, "{sub}");
        assert!(stdout(&o).starts_with(&format!("fastroute rules {sub} - ")), "{sub}: {}", stdout(&o));
        assert_eq!(stdout(&run(&["help", "rules", sub])), stdout(&o), "{sub}");
    }
    let o = rules(&["wat"]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("unknown command 'rules wat'"), "{}", stderr(&o));
    assert_eq!(stdout(&run(&["help", "rules"])), stdout(&rules(&[])));
    assert!(stdout(&run(&["--help"])).contains("rules "));
}

// ---- the keyword prototype: the cases the design is about ----------------------------------

#[test]
fn usb_differential_pair_with_gap_skew_and_a_stated_impedance() {
    let d = generate("usb-diff.txt");
    assert_eq!(d["created_by"]["kind"], "script");
    let (r1, r2) = (rule(&d, "R1"), rule(&d, "R2"));
    assert_eq!((r1["kind"].as_str(), r1["strength"].as_str()), (Some("diff_pair"), Some("hard")));
    assert_eq!(r1["applies_to"], json!({"nets": ["USB_DP", "USB_DN"]}), "\"USB D+/D-\" was resolved to the pair of the design");
    assert_eq!(r1["params"], json!({"gap_mm": 0.15, "skew_mm": 0.1}));
    // "should" makes the impedance soft; the number 90 is quoted from the text, not made up
    assert_eq!((r2["kind"].as_str(), r2["strength"].as_str(), r2["params"].clone()), (Some("impedance"), Some("soft"), json!({"target_ohm": 90.0, "differential": true})));
    assert!(r2["evidence"][0]["quote"].as_str().unwrap().contains("90 ohm"));
    // every rule points at words of the input, and at design data
    for r in [r1, r2] {
        assert_eq!(r["evidence"][0]["kind"], "natural_language");
        assert!(d["inputs"][0]["text"].as_str().unwrap().contains(r["evidence"][0]["quote"].as_str().unwrap()));
    }
    let (c, rep) = check("usb-diff.draft.json");
    assert_eq!(c, 0);
    assert_eq!((rep["valid"].clone(), rep["summary"]["executable"].clone()), (json!(true), json!(2)));
    assert_eq!(rep["rules"][0]["capability"], "supported");
    assert_eq!(rep["rules"][1]["capability"], "via_kicad");
}

#[test]
fn widening_power_nets_and_a_net_width_with_the_number_before_the_word() {
    let d = generate("power-widen.txt");
    let (r1, r2) = (rule(&d, "R1"), rule(&d, "R2"));
    assert_eq!((r1["kind"].as_str(), r1["strength"].as_str(), r1["params"].clone()), (Some("trace_width"), Some("hard"), json!({"width_mm": 0.5})));
    assert_eq!(r1["applies_to"], json!({"nets": ["VBUS"]}), "\"power\" is read as the supply nets of the design, and says so");
    assert!(d["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("\"power\" was read as the nets VBUS")));
    assert_eq!((r2["strength"].as_str(), r2["params"].clone(), r2["applies_to"].clone()), (Some("soft"), json!({"width_mm": 0.4}), json!({"nets": ["GND"]})));
    // wider than the design's 0.25 mm: nothing is loosened
    assert!(r1.get("relaxes").is_none() && r2.get("relaxes").is_none());
    assert_eq!(check("power-widen.draft.json").0, 0);
}

#[test]
fn a_layer_preference_is_unsupported_and_never_executable() {
    let d = generate("layer-preference.txt");
    let r = rule(&d, "R1");
    assert_eq!((r["kind"].as_str(), r["strength"].as_str()), (Some("layer_preference"), Some("unsupported")));
    assert_eq!(r["params"], json!({"preferred_layers": ["B.Cu", "F.Cu"]}), "\"top layer\" / \"bottom layer\" resolved to the layer names of the design");
    let (c, rep) = check("layer-preference.draft.json");
    assert_eq!(c, 0);
    assert_eq!((rep["rules"][0]["executable"].clone(), rep["rules"][0]["capability"].clone(), rep["summary"]["executable"].clone()), (json!(false), json!("unsupported"), json!(0)));
    assert!(rep["rules"][0]["reason"].as_str().unwrap().contains("cannot enforce"), "{}", rep["rules"][0]["reason"]);
}

#[test]
fn insufficient_information_stays_unknown_and_says_what_is_missing() {
    let d = generate("insufficient-info.txt");
    let rules_: Vec<&Value> = d["rules"].as_array().unwrap().iter().collect();
    assert_eq!(rules_.len(), 2);
    let kinds: Vec<&str> = rules_.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["timing_delay", "current_capacity"]);
    for r in &rules_ {
        assert_eq!(r["strength"], "unknown");
        assert!(r.get("params").is_none(), "an unknown rule carries no value: {r}");
        assert!(!r["needed"].as_array().unwrap().is_empty(), "{r}");
    }
    assert!(rules_[1]["needed"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("current in A")));
    // the sentence that could not be read is reported, not turned into a rule
    assert!(d["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("not understood") && n.as_str().unwrap().contains("Keep the design quiet")));
    let (c, rep) = check("insufficient-info.draft.json");
    assert_eq!((c, rep["summary"]["executable"].clone(), rep["summary"]["unknown"].clone()), (0, json!(0), json!(2)));
    // text without any requirement gives no rule and no invention
    let t = tmp("nothing");
    std::fs::write(t.join("t.txt"), "Please make it nice.").unwrap();
    let o = rules(&["draft", "--text-file", &s(&t.join("t.txt")), "-o", &s(&t.join("d.json")), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(json(&t.join("d.json"))["rules"], json!([]));
}

#[test]
fn asking_to_loosen_a_hard_constraint_is_declared_and_needs_an_explicit_acceptance() {
    let d = generate("relax-hard-constraint.txt");
    let r = rule(&d, "R1");
    assert_eq!((r["kind"].as_str(), r["params"].clone()), (Some("clearance"), json!({"clearance_mm": 0.1})));
    // the generator states what it found: the design requires 0.2 mm
    assert_eq!(r["relaxes"]["parameter"], "clearance_mm");
    assert_eq!(r["relaxes"]["baseline"], 0.2);
    let (c, rep) = check("relax-hard-constraint.draft.json");
    assert_eq!((c, rep["summary"]["relaxations"].clone()), (0, json!(1)));
    let t = tmp("relax-flow");
    let conf = t.join("conf.json");
    let o = rules(&["confirm-template", "relax-hard-constraint.draft.json", "--design", "design-inspect.json", "-o", &s(&conf)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let mut c = json(&conf);
    assert_eq!(c["accepted_relaxations"], json!(["R1"]), "the form lists the loosening for the person to decide on");
    c["confirmed_by"]["name"] = json!("Alice Example");
    // a person who accepts the rule but not the loosening gets no output
    c["accepted_relaxations"] = json!([]);
    std::fs::write(&conf, serde_json::to_vec(&c).unwrap()).unwrap();
    let out = t.join("out");
    let o = rules(&["apply", "relax-hard-constraint.draft.json", "--confirmation", &s(&conf), "--design", "design-inspect.json", "-o", &s(&out)]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("loosens a limit of the design") && stderr(&o).contains("accepted_relaxations"), "{}", stderr(&o));
    assert!(!out.exists(), "nothing is written for a confirmation that does not match");
    // explicitly accepted: the KiCad rule is written, with the value as asked
    c["accepted_relaxations"] = json!(["R1"]);
    std::fs::write(&conf, serde_json::to_vec(&c).unwrap()).unwrap();
    let o = rules(&["apply", "relax-hard-constraint.draft.json", "--confirmation", &s(&conf), "--design", "design-inspect.json", "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let dru = std::fs::read_to_string(out.join("rules.kicad_dru")).unwrap();
    assert!(dru.starts_with("(version 1)") && dru.contains("(rule \"R1\"") && dru.contains("(constraint clearance (min 0.1mm))"), "{dru}");
    assert_eq!(json(&out.join("apply-report.json"))["relaxations_accepted"], json!(["R1"]));
    // KiCad's DRC checks a .kicad_dru; the router gets only some of its rules. Say so: with a real
    // KiCad 8.0.9 the plugin printed "NOT applied" for clearance / track_width rules written in
    // this form, while the file said it "carries them into the DSN"
    let next = json(&out.join("apply-report.json"))["next_steps"].to_string();
    assert!(next.contains("NOT applied") && next.contains("checked, not obeyed"), "{next}");
    assert!(!next.contains("carries its rules into the DSN"), "{next}");
    assert!(dru.contains("prints NOT applied for the rest") && !dru.contains("carries them into the DSN"), "{dru}");
}

#[test]
fn the_committed_examples_are_what_the_tools_write() {
    // design data: inspect.json of usb_demo.dsn
    let t = tmp("drift");
    let out = t.join("inspect.json");
    let o = rules_run_inspect(&out);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let (mut now, mut committed) = (json(&out), json(&examples().join("design-inspect.json")));
    for v in [&mut now, &mut committed] {
        v["fastroute_version"] = json!("x");
    }
    assert_eq!(now, committed, "docs/examples/rules/design-inspect.json is out of date");
    // the generated drafts
    for name in ["usb-diff", "power-widen", "layer-preference", "insufficient-info", "relax-hard-constraint"] {
        let generated = generate(&format!("{name}.txt"));
        let committed = json(&examples().join(format!("{name}.draft.json")));
        assert_eq!(generated, committed, "docs/examples/rules/{name}.draft.json is out of date (regenerate it with `fastroute rules draft`)");
    }
}

fn rules_run_inspect(out: &Path) -> std::process::Output {
    run_in(&examples(), &["inspect", "-i", "usb_demo.dsn", "-o", &s(out), "-q"])
}

// ---- drafts written by someone else (an AI model): every misuse is stopped -------------------

#[test]
fn drafts_that_misbehave_are_refused_with_the_reason() {
    // (file, the error code that must be reported)
    let cases = [
        ("bad-silent-relaxation", "silent_relaxation"),
        ("bad-declared-baseline-wrong", "relaxation_mismatch"),
        ("bad-guessed-impedance", "value_without_source"),
        ("bad-geometry", "geometry_in_draft"),
        ("bad-hard-unsupported", "cannot_enforce"),
        ("bad-fabricated-quote", "quote_not_found"),
        ("bad-unknown-with-value", "unknown_with_value"),
        ("bad-unknown-net", "unknown_net"),
        ("bad-version", "unsupported_version"),
    ];
    for (name, want) in cases {
        let (c, rep) = check(&format!("{name}.draft.json"));
        assert_eq!(c, 3, "{name} must not be valid");
        assert_eq!(rep["valid"], false, "{name}");
        assert!(codes(&rep, "errors").contains(&want.to_string()), "{name}: expected {want}, got {:?}", codes(&rep, "errors"));
        assert!(rep["errors"].as_array().unwrap().iter().all(|e| !e["message"].as_str().unwrap().is_empty() && e["path"].as_str().unwrap().starts_with('$')));
    }
    // the messages say what to do
    let (_, rep) = check("bad-silent-relaxation.draft.json");
    let m = rep["errors"][0]["message"].as_str().unwrap();
    assert!(m.contains("from 0.2 to 0.1") && m.contains("declare it with `relaxes`"), "{m}");
    let (_, rep) = check("bad-guessed-impedance.draft.json");
    assert!(rep["errors"][0]["message"].as_str().unwrap().contains("cannot be read from a board or a picture"), "{}", rep["errors"][0]["message"]);
    let (_, rep) = check("bad-geometry.draft.json");
    assert!(rep["errors"].as_array().unwrap().iter().any(|e| e["path"] == "$.rules[0].params.points"));
    // and the two good ones from the same author pass
    for ok in ["ok-ai-clearance", "ok-length-match"] {
        let (c, rep) = check(&format!("{ok}.draft.json"));
        assert_eq!((c, rep["valid"].clone(), rep["summary"]["executable"].clone()), (0, json!(true), json!(1)), "{ok}: {:?}", rep["errors"]);
    }
}

#[test]
fn a_draft_is_checked_against_the_design_it_is_for() {
    // without the design the checks that need it are named as not done (and apply refuses)
    let o = rules(&["check", "bad-silent-relaxation.draft.json", "-q"]);
    let rep: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((code(&o), rep["design_checked"].clone()), (0, json!(false)));
    assert!(codes(&rep, "warnings").contains(&"design_unchecked".to_string()) && codes(&rep, "warnings").contains(&"baseline_unknown".to_string()), "{:?}", rep["warnings"]);
    // a draft that carries the hash of another design is refused
    let t = tmp("mismatch");
    let mut d = json(&examples().join("usb-diff.draft.json"));
    d["inputs"][1]["sha256"] = json!("0".repeat(64));
    std::fs::write(t.join("d.json"), serde_json::to_vec(&d).unwrap()).unwrap();
    let o = rules(&["check", &s(&t.join("d.json")), "--design", "design-inspect.json", "-q"]);
    assert_eq!(code(&o), 3);
    assert!(codes(&serde_json::from_slice(&o.stdout).unwrap(), "errors").contains(&"design_mismatch".to_string()));
    // an inspect.json of a failed run, and things that are not one
    let bad = t.join("bad.json");
    std::fs::write(&bad, r#"{"format":"fastroute-inspect","format_version":"1.0","status":"error"}"#).unwrap();
    let o = rules(&["check", "usb-diff.draft.json", "--design", &s(&bad), "-q"]);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("failed run"), "{}", stderr(&o));
    std::fs::write(&bad, "{}").unwrap();
    assert_eq!(code(&rules(&["check", "usb-diff.draft.json", "--design", &s(&bad), "-q"])), 3);
    // and a draft that is not JSON
    std::fs::write(&bad, "nope").unwrap();
    let o = rules(&["check", &s(&bad), "-q"]);
    assert_eq!(code(&o), 3);
    assert!(stderr(&o).contains("not valid JSON"));
    assert_eq!(code(&rules(&["check", "/no/such.json", "-q"])), 3);
    assert_eq!(code(&rules(&["check", "-q"])), 2);
}

// ---- nothing is executed without a person's decision -------------------------------------

fn confirm(draft: &str, dir: &Path, name: &str) -> PathBuf {
    let conf = dir.join("conf.json");
    let o = rules(&["confirm-template", draft, "--design", "design-inspect.json", "-o", &s(&conf)]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    if !name.is_empty() {
        let mut c = json(&conf);
        c["confirmed_by"]["name"] = json!(name);
        std::fs::write(&conf, serde_json::to_vec(&c).unwrap()).unwrap();
    }
    conf
}

fn apply(draft: &str, conf: &Path, out: &Path, extra: &[&str]) -> std::process::Output {
    let mut a = vec!["apply", draft, "--confirmation", &*Box::leak(s(conf).into_boxed_str()), "--design", "design-inspect.json", "-o", &*Box::leak(s(out).into_boxed_str())];
    a.extend(extra);
    rules(&a)
}

#[test]
fn apply_writes_only_what_a_named_person_confirmed() {
    let t = tmp("apply");
    let out = t.join("out");
    // the form is empty of a name: refused
    let unnamed = tmp("apply-unnamed");
    let conf = confirm("usb-diff.draft.json", &unnamed, "");
    let o = apply("usb-diff.draft.json", &conf, &out, &[]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("confirmed_by.name is empty"), "{}", stderr(&o));
    assert!(!out.exists());
    // named: written
    let conf = confirm("usb-diff.draft.json", &t, "Alice Example");
    let o = apply("usb-diff.draft.json", &conf, &out, &["-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let report = json(&out.join("apply-report.json"));
    assert_eq!(report["format"], "fastroute-rule-apply");
    assert_eq!(report["confirmed_by"], "Alice Example");
    assert_eq!(report["executed"].as_array().unwrap().len(), 2);
    let pairs = std::fs::read_to_string(out.join("pairs.txt")).unwrap();
    assert!(pairs.lines().any(|l| l == "pair USB_DP USB_DN gap=0.15 skew=0.1"), "{pairs}");
    assert_eq!(std::fs::read_to_string(out.join("route-args.txt")).unwrap(), "--pairs=pairs.txt\n");
    // the impedance is a manual step with the real command, not something that was "applied"
    assert!(report["manual_steps"][0].as_str().unwrap().contains("impedance_cli.py") && report["manual_steps"][0].as_str().unwrap().contains("=90d"));
    assert!(report["acceptance"].as_str().unwrap().contains("DRC"));
    assert!(report["next_steps"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("--require-drc")));
    // the written pairs file is one the engine reads: route the design with it
    let o = run_in(&t, &["route", &s(&examples().join("usb_demo.dsn")), "-o", &s(&t.join("r.ses")), "--pairs", &s(&out.join("pairs.txt")), "--max-time=60", "--router.optimizer.enabled=false", "-q", "--allow-partial"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert!(std::fs::read_to_string(t.join("r.ses")).unwrap().starts_with("(session"));
}

#[test]
fn a_length_match_rule_becomes_a_tune_file_the_engine_reads() {
    let t = tmp("tune");
    let conf = confirm("ok-length-match.draft.json", &t, "Alice Example");
    let out = t.join("out");
    let o = apply("ok-length-match.draft.json", &conf, &out, &["-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let tune = std::fs::read_to_string(out.join("tune.txt")).unwrap();
    assert!(tune.contains("group R1 tolerance=0.3\n  USB_DP\n  USB_DN\n"), "{tune}");
    let o = run_in(&t, &["route", &s(&examples().join("usb_demo.dsn")), "-o", &s(&t.join("r.ses")), "--tune", &s(&out.join("tune.txt")), "--max-time=60", "--router.optimizer.enabled=false", "-q", "--allow-partial"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
}

#[test]
fn unknown_and_unsupported_rules_never_produce_output() {
    for draft in ["insufficient-info.draft.json", "layer-preference.draft.json"] {
        let t = tmp("never");
        let conf = confirm(draft, &t, "Alice Example");
        let c = json(&conf);
        assert_eq!(c["accepted_rules"], json!([]), "{draft}: nothing in them can be accepted");
        assert!(!c["acknowledged_not_executed"].as_array().unwrap().is_empty());
        let out = t.join("out");
        let o = apply(draft, &conf, &out, &["-q"]);
        assert_eq!(code(&o), 0, "{draft}: {}", stderr(&o));
        let mut names: Vec<String> = std::fs::read_dir(&out).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        assert_eq!(names, ["apply-report.json"], "{draft}: only the report, no rule file");
        let r = json(&out.join("apply-report.json"));
        assert_eq!(r["executed"], json!([]));
        assert_eq!(r["not_executed"].as_array().unwrap().len(), c["acknowledged_not_executed"].as_array().unwrap().len());
        assert!(r["not_executed"].as_array().unwrap().iter().all(|n| !n["reason"].as_str().unwrap().is_empty()));
        // accepting one of them anyway is refused
        let mut forced = c.clone();
        let first = forced["acknowledged_not_executed"][0].clone();
        forced["accepted_rules"] = json!([first]);
        std::fs::write(&conf, serde_json::to_vec(&forced).unwrap()).unwrap();
        let o = apply(draft, &conf, &t.join("out2"), &[]);
        assert_eq!(code(&o), 3, "{draft}: {}", stderr(&o));
        assert!(stderr(&o).contains("cannot be accepted") || stderr(&o).contains("both accepted"), "{}", stderr(&o));
        assert!(!t.join("out2").exists());
    }
}

#[test]
fn a_confirmation_must_match_the_draft_exactly() {
    let t = tmp("mismatch-conf");
    let conf = confirm("usb-diff.draft.json", &t, "Alice Example");
    let base = json(&conf);
    let try_conf = |edit: &dyn Fn(&mut Value), needle: &str| {
        let mut c = base.clone();
        edit(&mut c);
        let p = t.join("c.json");
        std::fs::write(&p, serde_json::to_vec(&c).unwrap()).unwrap();
        let out = t.join(format!("o-{}", needle.len()));
        let o = apply("usb-diff.draft.json", &p, &out, &[]);
        assert_eq!(code(&o), 3, "{needle}: {}", stderr(&o));
        assert!(stderr(&o).contains(needle), "{needle} in {}", stderr(&o));
        assert!(!out.exists(), "{needle}: nothing may be written");
    };
    try_conf(&|c| c["draft_sha256"] = json!("0".repeat(64)), "for another draft");
    try_conf(&|c| c["accepted_rules"] = json!(["R1"]), "neither accepted nor acknowledged");
    try_conf(&|c| c["acknowledged_not_executed"] = json!(["R2"]), "both accepted and acknowledged");
    try_conf(&|c| c["accepted_rules"] = json!(["R1", "R2", "R9"]), "the draft does not have");
    try_conf(&|c| c["confirmed_by"]["name"] = json!("   "), "confirmed_by.name is empty");
    try_conf(&|c| c["format"] = json!("something-else"), "format");
    try_conf(&|c| { c.as_object_mut().unwrap().remove("accepted_rules"); }, "accepted_rules");
    // a draft that changed after it was confirmed needs a new confirmation
    let edited = t.join("edited.json");
    let mut d = json(&examples().join("usb-diff.draft.json"));
    d["rules"][0]["params"]["gap_mm"] = json!(0.05);
    std::fs::write(&edited, serde_json::to_vec_pretty(&d).unwrap()).unwrap();
    let o = apply(&s(&edited), &conf, &t.join("o-edited"), &[]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("for another draft") && stderr(&o).contains("confirmed again"), "{}", stderr(&o));
    // an invalid draft is never applied, whatever the confirmation says
    let o = apply("bad-silent-relaxation.draft.json", &conf, &t.join("o-bad"), &[]);
    assert_eq!(code(&o), 3);
    assert!(!t.join("o-bad").exists());
}

#[test]
fn apply_needs_everything_and_protects_its_output() {
    let t = tmp("apply-guard");
    let conf = confirm("usb-diff.draft.json", &t, "Alice Example");
    // no confirmation, no design
    let o = rules(&["apply", "usb-diff.draft.json", "--design", "design-inspect.json", "-o", &s(&t.join("o1"))]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("nothing is applied without a person's decision"), "{}", stderr(&o));
    let o = rules(&["apply", "usb-diff.draft.json", "--confirmation", &s(&conf), "-o", &s(&t.join("o2"))]);
    assert_eq!(code(&o), 2);
    assert!(stderr(&o).contains("needs the design"), "{}", stderr(&o));
    // an existing non-empty directory is refused; --force replaces only the files of the command
    let out = t.join("out");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(out.join("notes.txt"), "mine").unwrap();
    std::fs::write(out.join("pairs.txt"), "old").unwrap();
    let o = apply("usb-diff.draft.json", &conf, &out, &[]);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(std::fs::read(out.join("pairs.txt")).unwrap(), b"old");
    assert_eq!(code(&apply("usb-diff.draft.json", &conf, &out, &["--force", "-q"])), 0);
    assert!(std::fs::read_to_string(out.join("pairs.txt")).unwrap().contains("pair USB_DP USB_DN"));
    assert_eq!(std::fs::read(out.join("notes.txt")).unwrap(), b"mine");
    // the form is not overwritten without --force either
    let o = rules(&["confirm-template", "usb-diff.draft.json", "--design", "design-inspect.json", "-o", &s(&conf)]);
    assert_eq!(code(&o), 4);
    assert!(stderr(&o).contains("--force"));
    // an invalid draft has nothing to confirm
    let o = rules(&["confirm-template", "bad-geometry.draft.json", "--design", "design-inspect.json", "-o", &s(&t.join("x.json"))]);
    assert_eq!(code(&o), 3);
    assert!(!t.join("x.json").exists());
}

#[test]
fn draft_output_rules_and_inputs() {
    let t = tmp("draft-io");
    let out = t.join("d.json");
    let ok = ["draft", "--text", "Widen VBUS to 0.5 mm.", "--design", "design-inspect.json", "-o", &*s(&out)];
    assert_eq!(code(&rules(&ok)), 0);
    let first = file_hash(&out);
    let o = rules(&ok);
    assert_eq!(code(&o), 4, "{}", stderr(&o));
    assert_eq!(file_hash(&out), first);
    // to standard output
    let o = rules(&["draft", "--text", "Widen VBUS to 0.5 mm.", "--design", "design-inspect.json", "-o", "-", "-q"]);
    assert_eq!(code(&o), 0);
    assert_eq!(serde_json::from_slice::<Value>(&o.stdout).unwrap()["rules"][0]["params"], json!({"width_mm": 0.5}));
    // usage and input errors
    for (args, want, text) in [
        (vec!["draft", "-o", "x.json"], 2, "no requirements"),
        (vec!["draft", "--text", "a", "--text-file", "b", "-o", "x"], 2, "either --text or --text-file"),
        (vec!["draft", "--text", "a"], 2, "no output file"),
        (vec!["draft", "--text", "   ", "-o", "-"], 3, "empty"),
        (vec!["draft", "--text-file", "/no/such.txt", "-o", "-"], 3, "/no/such.txt"),
        (vec!["draft", "--text", "a", "--ai-pack", "x", "--design", "y", "-o", "-"], 2, "either --ai-pack or --design"),
        (vec!["draft", "--text", "a", "--ai-pack", "/no/such/pack", "-o", "-"], 3, "manifest.json"),
    ] {
        let o = rules(&args);
        assert_eq!(code(&o), want, "{args:?}: {}", stderr(&o));
        assert!(stderr(&o).contains(text), "{args:?}: {}", stderr(&o));
    }
    // without the design a pair cannot be resolved: unknown, with what is missing - not a guess
    let o = rules(&["draft", "--text", "The USB differential pair must have a gap of 0.15 mm.", "-o", "-", "-q"]);
    let d: Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!((d["rules"][0]["kind"].as_str(), d["rules"][0]["strength"].as_str()), (Some("diff_pair"), Some("unknown")));
    assert!(d["rules"][0]["needed"][0].as_str().unwrap().contains("two nets"));
}

#[test]
fn an_ai_pack_is_a_design_too_and_a_damaged_one_is_refused() {
    let t = tmp("pack-design");
    let pack = t.join("pack");
    let o = run(&["ai-pack", "-i", &s(&examples().join("usb_demo.dsn")), "-o", &s(&pack), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = t.join("d.json");
    let o = rules(&["draft", "--text-file", "usb-diff.txt", "--ai-pack", &s(&pack), "-o", &s(&out), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let d = json(&out);
    assert_eq!(d["inputs"][1]["kind"], "ai_pack");
    assert_eq!(rule(&d, "R1")["params"], json!({"gap_mm": 0.15, "skew_mm": 0.1}));
    let o = rules(&["check", &s(&out), "--ai-pack", &s(&pack), "-q"]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    // the pack's inspect.json is changed behind the manifest's back: the pack is refused
    let mut inspect = std::fs::read_to_string(pack.join("inspect.json")).unwrap();
    inspect = inspect.replace("USB_DP", "USB_XX");
    std::fs::write(pack.join("inspect.json"), inspect).unwrap();
    let o = rules(&["check", &s(&out), "--ai-pack", &s(&pack), "-q"]);
    assert_eq!(code(&o), 3, "{}", stderr(&o));
    assert!(stderr(&o).contains("not consistent"), "{}", stderr(&o));
}

#[test]
fn everything_runs_without_network_or_an_ai_service() {
    // an empty PATH and no environment: the whole flow is local
    let t = tmp("offline");
    let conf = confirm("ok-length-match.draft.json", &t, "Alice Example");
    assert_eq!(code(&apply("ok-length-match.draft.json", &conf, &t.join("o"), &["-q"])), 0);
    let src = std::fs::read_to_string(manifest_dir().join("src/rules/mod.rs")).unwrap();
    for banned in ["TcpStream", "reqwest", "curl", "api.openai", "api.anthropic", "std::net::"] {
        assert!(!src.contains(banned), "the rules module must stay offline: {banned}");
    }
}
