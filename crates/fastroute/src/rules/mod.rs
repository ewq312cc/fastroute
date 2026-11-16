//! `fastroute rules`: rule drafts from people, scripts or AI models - checked, confirmed, applied.
//!
//! ```text
//! rules schema              the JSON Schema of a draft (for tools that generate drafts)
//! rules capabilities        what each kind of rule maps to in fastroute / KiCad, or why not
//! rules draft               a draft from written requirements, offline (keyword prototype)
//! rules check DRAFT         the deterministic checker; exit 3 if the draft is not valid
//! rules confirm-template    the confirmation form a person fills in (with their name)
//! rules apply DRAFT         pairs / tune / .kicad_dru files for the rules a person confirmed
//! ```
//!
//! Nothing here calls an AI service or the network; an AI model, if one is used, is an external
//! tool that writes a draft file. Design and trust boundaries: `docs/AI_RULES.md`.

mod apply;
mod check;
mod facts;
mod kinds;
mod nl;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::cmd::{check_output, opt, write_atomically, write_output, CmdError, Opt, Parsed, Spec};
use crate::load::sha256_hex;
use facts::DesignFacts;

const DRAFT_SCHEMA: &str = include_str!("../../../../docs/schema/rule-draft-1.0.schema.json");
const CONFIRMATION_SCHEMA: &str = include_str!("../../../../docs/schema/rule-confirmation-1.0.schema.json");

const DESIGN_OPTS: [Opt; 2] = [
    opt("ai-pack", None, Some("DIR"), "the design: an ai-pack directory (its inspect.json, after verifying the pack)"),
    opt("design", None, Some("FILE"), "the design: an inspect.json"),
];

const SCHEMA_OPTS: &[Opt] = &[opt("confirmation", None, None, "print the schema of the confirmation instead")];
const NO_OPTS: &[Opt] = &[];
const DRAFT_OPTS: &[Opt] = &[
    opt("text", None, Some("TEXT"), "the requirements, as text"),
    opt("text-file", None, Some("FILE"), "the requirements, in a text file"),
    DESIGN_OPTS[0],
    DESIGN_OPTS[1],
    opt("output", Some('o'), Some("FILE"), "draft to write ('-' = standard output)"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary"),
];
const CHECK_OPTS: &[Opt] = &[
    opt("draft", None, Some("FILE"), "the draft (also the first positional argument)"),
    DESIGN_OPTS[0],
    DESIGN_OPTS[1],
    opt("output", Some('o'), Some("FILE"), "write the check report here instead of standard output"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];
const CONFIRM_OPTS: &[Opt] = &[
    opt("draft", None, Some("FILE"), "the draft (also the first positional argument)"),
    DESIGN_OPTS[0],
    DESIGN_OPTS[1],
    opt("output", Some('o'), Some("FILE"), "confirmation form to write"),
    opt("force", None, None, "replace an existing output file"),
];
const APPLY_OPTS: &[Opt] = &[
    opt("draft", None, Some("FILE"), "the draft (also the first positional argument)"),
    opt("confirmation", None, Some("FILE"), "the confirmation of exactly this draft, with the name of the person who decided"),
    DESIGN_OPTS[0],
    DESIGN_OPTS[1],
    opt("output-dir", Some('o'), Some("DIR"), "directory for pairs.txt, tune.txt, rules.kicad_dru, route-args.txt and apply-report.json"),
    opt("force", None, None, "replace those files in an existing directory (nothing else is touched)"),
    opt("quiet", Some('q'), None, "no summary"),
];

const fn spec(name: &'static str, about: &'static str, usage: &'static str, opts: &'static [Opt], footer: &'static str) -> Spec {
    Spec { name, about, usage, opts, passthrough: &[], footer }
}

const SCHEMA_SPEC: Spec = spec("rules schema", "print the JSON Schema of a rule draft", "rules schema [--confirmation]", SCHEMA_OPTS, "The same file is docs/schema/rule-draft-1.0.schema.json; the checker enforces it.");
const CAP_SPEC: Spec = spec("rules capabilities", "print what each kind of rule maps to, or why it cannot be applied", "rules capabilities", NO_OPTS, "capability: supported (a fastroute input is written), via_kicad (through KiCad's rules / plugin tools),\nneeds_core (the engine has the primitives but nothing reaches them yet), unsupported (the engine cannot).");
const DRAFT_SPEC: Spec = spec(
    "rules draft",
    "make a rule draft from written requirements, offline (a keyword prototype, not an AI)",
    "rules draft (--text TEXT | --text-file FILE) [--ai-pack DIR | --design inspect.json] --output draft.json",
    DRAFT_OPTS,
    "Every rule quotes the sentence it comes from. What cannot be read becomes an `unknown` rule that says what is missing,\nwhat the engine cannot enforce becomes `unsupported`; nothing is guessed. Give the design to resolve net names, layers\nand to declare when a requirement would loosen a limit of the design. An AI model can write the same file instead.",
);
const CHECK_SPEC: Spec = spec(
    "rules check",
    "check a rule draft: structure, evidence, values, capability, loosening of the design's limits",
    "rules check DRAFT [--ai-pack DIR | --design inspect.json]",
    CHECK_OPTS,
    "Exit 0 if the draft is valid (warnings are listed), 3 if it is not. The JSON report on standard output says, per rule,\nwhether it can be executed and why not. Without the design, nets, layers and loosening cannot be checked (a warning).",
);
const CONFIRM_SPEC: Spec = spec(
    "rules confirm-template",
    "write the confirmation form for a draft (a person fills in their name)",
    "rules confirm-template DRAFT [--ai-pack DIR | --design inspect.json] --output confirmation.json",
    CONFIRM_OPTS,
    "The form lists every rule that would be executed, every loosening, and every rule that will NOT be executed. The name is\nleft empty on purpose: `rules apply` refuses a confirmation without it.",
);
const APPLY_SPEC: Spec = spec(
    "rules apply",
    "write fastroute / KiCad inputs for the rules a person confirmed",
    "rules apply DRAFT --confirmation confirmation.json (--ai-pack DIR | --design inspect.json) --output-dir DIR",
    APPLY_OPTS,
    "Needs the design (loosening is checked against it), a valid draft and a confirmation of exactly this draft (SHA-256).\nOnly valid, supported, accepted hard / soft rules produce output; unknown and unsupported rules never do. It routes\nnothing: the result is accepted by the real DRC of the routed board (fastroute ai-pack --kicad-pcb ... --require-drc).",
);

const SUBCOMMANDS: &str = "\
commands:
  schema              print the JSON Schema of a rule draft
  capabilities        what each kind of rule maps to in fastroute / KiCad, or why not
  draft               make a draft from written requirements, offline (keyword prototype)
  check               check a draft (exit 3 if it is not valid)
  confirm-template    write the confirmation form for a draft
  apply               write fastroute / KiCad inputs for the rules a person confirmed

Run `fastroute rules <command> --help` for the options. Design and trust boundaries: docs/AI_RULES.md.
";

pub fn help_text() -> String {
    format!("fastroute rules - rule drafts from people, scripts or AI models: checked, confirmed, applied\n\nusage: fastroute rules <command> [options]\n\n{SUBCOMMANDS}")
}

pub fn run(args: &[String]) -> Result<(), CmdError> {
    let Some((sub, rest)) = args.split_first() else {
        print!("{}", help_text());
        return Ok(());
    };
    match sub.as_str() {
        "-h" | "--help" | "help" => {
            print!("{}", help_text());
            Ok(())
        }
        "schema" => cmd_schema(rest),
        "capabilities" => cmd_capabilities(rest),
        "draft" => cmd_draft(rest),
        "check" => cmd_check(rest),
        "confirm-template" => cmd_confirm_template(rest),
        "apply" => cmd_apply(rest),
        other => Err(CmdError::usage(format!("unknown command 'rules {other}' (schema, capabilities, draft, check, confirm-template, apply)"))),
    }
}

/// `fastroute help rules [command]`.
pub fn help_for(sub: Option<&str>) -> String {
    match sub {
        Some("schema") => SCHEMA_SPEC.help_text(),
        Some("capabilities") => CAP_SPEC.help_text(),
        Some("draft") => DRAFT_SPEC.help_text(),
        Some("check") => CHECK_SPEC.help_text(),
        Some("confirm-template") => CONFIRM_SPEC.help_text(),
        Some("apply") => APPLY_SPEC.help_text(),
        _ => help_text(),
    }
}

fn parse(spec: &Spec, args: &[String]) -> Result<Option<Parsed>, CmdError> {
    let p = spec.parse(args)?;
    if p.help {
        print!("{}", spec.help_text());
        return Ok(None);
    }
    Ok(Some(p))
}

fn draft_schema() -> Value {
    serde_json::from_str(DRAFT_SCHEMA).expect("the published schema is valid JSON")
}

fn cmd_schema(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&SCHEMA_SPEC, args)? else { return Ok(()) };
    print!("{}", if p.flag("confirmation") { CONFIRMATION_SCHEMA } else { DRAFT_SCHEMA });
    Ok(())
}

fn cmd_capabilities(args: &[String]) -> Result<(), CmdError> {
    let Some(_) = parse(&CAP_SPEC, args)? else { return Ok(()) };
    let kinds: Vec<Value> = kinds::KINDS
        .iter()
        .map(|k| {
            json!({
                "kind": k.name,
                "capability": k.capability,
                "executable": k.capability.executable(),
                "maps_to": k.maps_to,
                "required_params": k.required,
                "one_of_params": k.one_of,
                "params": k.params.iter().map(|p| json!({"key": p.key, "type": p.ty})).collect::<Vec<_>>(),
                "value_must_be_stated_by_a_person": k.explicit_source,
            })
        })
        .collect();
    println!("{}", serde_json::to_string_pretty(&json!({"format": "fastroute-rule-capabilities", "format_version": "1.0", "fastroute_version": env!("CARGO_PKG_VERSION"), "kinds": kinds})).expect("serialises"));
    Ok(())
}

/// The design from `--ai-pack` / `--design`: facts, and (kind, path) for the draft's inputs.
fn design_from(p: &Parsed) -> Result<Option<(DesignFacts, &'static str, PathBuf)>, CmdError> {
    match (p.path("ai-pack"), p.path("design")) {
        (Some(_), Some(_)) => Err(CmdError::usage("give either --ai-pack or --design, not both")),
        (Some(dir), None) => Ok(Some((DesignFacts::from_ai_pack(&dir)?, "ai_pack", dir))),
        (None, Some(f)) => Ok(Some((DesignFacts::from_inspect_file(&f)?, "inspect_json", f))),
        (None, None) => Ok(None),
    }
}

fn draft_path(p: &Parsed) -> Result<PathBuf, CmdError> {
    match (p.path("draft"), p.positional.as_slice()) {
        (Some(d), []) => Ok(d),
        (None, [one]) => Ok(PathBuf::from(one)),
        (Some(_), _) => Err(CmdError::usage(format!("unexpected argument '{}': the draft is given with --draft", p.positional[0]))),
        (None, []) => Err(CmdError::usage("no draft file")),
        (None, _) => Err(CmdError::usage("more than one draft given")),
    }
}

/// The draft file: its bytes' hash and the parsed JSON.
fn read_draft(path: &Path) -> Result<(String, Value), CmdError> {
    let bytes = std::fs::read(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| CmdError::input(format!("{}: not valid JSON: {e}", path.display())))?;
    Ok((sha256_hex(&bytes), v))
}

fn print_findings(report: &check::CheckReport) {
    for f in &report.errors {
        eprintln!("error[{}] {}: {}", f.code, f.path, f.message);
    }
    for f in &report.warnings {
        eprintln!("warning[{}] {}: {}", f.code, f.path, f.message);
    }
}

fn cmd_draft(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&DRAFT_SPEC, args)? else { return Ok(()) };
    let text = match (p.value("text"), p.path("text-file")) {
        (Some(t), None) => t.to_string(),
        (None, Some(f)) => std::fs::read_to_string(&f).map_err(|e| CmdError::input(format!("{}: {e}", f.display())))?,
        (None, None) => return Err(CmdError::usage("no requirements (--text TEXT or --text-file FILE)")),
        (Some(_), Some(_)) => return Err(CmdError::usage("give either --text or --text-file, not both")),
    };
    if text.trim().is_empty() {
        return Err(CmdError::input("the requirements are empty"));
    }
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output file (--output draft.json, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    let mut inputs: Vec<&Path> = Vec::new();
    let tf = p.path("text-file");
    inputs.extend(tf.as_deref());
    if !to_stdout {
        check_output(&output, &inputs, p.flag("force"))?;
    }
    let design = design_from(&p)?;
    let generated = nl::generate(&text, design.as_ref().map(|d| &d.0), design.as_ref().map(|d| (d.1, d.2.to_str().unwrap_or(""))));
    let mut json_text = serde_json::to_string_pretty(&generated.draft).expect("serialises");
    json_text.push('\n');
    // the generator is held to the same checker as everybody else
    let report = check::check(&generated.draft, &sha256_hex(json_text.as_bytes()), design.as_ref().map(|d| &d.0), &draft_schema());
    if !report.valid {
        print_findings(&report);
        return Err(CmdError::failed("the draft generator produced a draft that the checker refuses (a bug in the prototype)"));
    }
    if to_stdout {
        print!("{json_text}");
    } else {
        write_output(&output, json_text.as_bytes(), &inputs, p.flag("force"))?;
    }
    if !p.flag("quiet") {
        let s = &report.summary;
        eprintln!(
            "{}: {} rules: {} hard, {} soft, {} unknown (need more information), {} unsupported; {} executable",
            if to_stdout { "draft".to_string() } else { output.display().to_string() },
            s.rules, s.hard, s.soft, s.unknown, s.unsupported, s.executable
        );
        for n in &generated.notes {
            eprintln!("note: {n}");
        }
        for w in &report.warnings {
            eprintln!("warning[{}]: {}", w.code, w.message);
        }
    }
    Ok(())
}

fn cmd_check(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&CHECK_SPEC, args)? else { return Ok(()) };
    let path = draft_path(&p)?;
    let output = p.path("output");
    if let Some(o) = &output {
        check_output(o, &[&path], p.flag("force"))?;
    }
    let (sha, draft) = read_draft(&path)?;
    let design = design_from(&p)?;
    let report = check::check(&draft, &sha, design.as_ref().map(|d| &d.0), &draft_schema());
    let mut json_text = serde_json::to_string_pretty(&report).expect("serialises");
    json_text.push('\n');
    match &output {
        Some(o) => write_output(o, json_text.as_bytes(), &[&path], p.flag("force"))?,
        None => print!("{json_text}"),
    }
    if !p.flag("quiet") {
        print_findings(&report);
        let s = &report.summary;
        eprintln!(
            "{}: {} - {} rules: {} hard, {} soft, {} unknown, {} unsupported; {} executable, {} loosening the design",
            path.display(), if report.valid { "valid" } else { "NOT valid" }, s.rules, s.hard, s.soft, s.unknown, s.unsupported, s.executable, s.relaxations
        );
    }
    if report.valid {
        Ok(())
    } else {
        Err(CmdError::input(format!("{}: the draft is not valid ({} errors)", path.display(), report.errors.len())))
    }
}

fn cmd_confirm_template(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&CONFIRM_SPEC, args)? else { return Ok(()) };
    let path = draft_path(&p)?;
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output file (--output confirmation.json)"))?;
    check_output(&output, &[&path], p.flag("force"))?;
    let (sha, draft) = read_draft(&path)?;
    let design = design_from(&p)?;
    let report = check::check(&draft, &sha, design.as_ref().map(|d| &d.0), &draft_schema());
    if !report.valid {
        print_findings(&report);
        return Err(CmdError::input(format!("{}: the draft is not valid, there is nothing to confirm", path.display())));
    }
    let accepted: Vec<&str> = report.rules.iter().filter(|r| r.executable).map(|r| r.id.as_str()).collect();
    let relax: Vec<&str> = report.rules.iter().filter(|r| r.executable && r.relaxes).map(|r| r.id.as_str()).collect();
    let rest: Vec<&str> = report.rules.iter().filter(|r| !r.executable).map(|r| r.id.as_str()).collect();
    let form = json!({
        "format": apply::CONFIRMATION_FORMAT,
        "format_version": "1.0",
        "draft_sha256": sha,
        "confirmed_by": {"name": "", "note": "Fill in your name. This is your decision, not the tool's: read the rules, remove the ones you do not want from accepted_rules (and add them to acknowledged_not_executed), and check every relaxation."},
        "accepted_rules": accepted,
        "accepted_relaxations": relax,
        "acknowledged_not_executed": rest,
    });
    let mut text = serde_json::to_string_pretty(&form).expect("serialises");
    text.push('\n');
    write_output(&output, text.as_bytes(), &[&path], p.flag("force"))?;
    eprintln!(
        "{}: {} rules would be executed, {} loosen a limit of the design, {} will not be executed. The name is empty: `rules apply` refuses it until a person fills it in",
        output.display(), accepted.len(), relax.len(), rest.len()
    );
    Ok(())
}

const OWN_FILES: [&str; 5] = ["pairs.txt", "tune.txt", "rules.kicad_dru", "route-args.txt", "apply-report.json"];

fn cmd_apply(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&APPLY_SPEC, args)? else { return Ok(()) };
    let path = draft_path(&p)?;
    let conf_path = p.path("confirmation").ok_or_else(|| CmdError::usage("no confirmation (--confirmation confirmation.json): nothing is applied without a person's decision"))?;
    let dir = p.path("output-dir").ok_or_else(|| CmdError::usage("no output directory (--output-dir DIR)"))?;
    let force = p.flag("force");
    let design = design_from(&p)?.ok_or_else(|| CmdError::usage("apply needs the design (--ai-pack DIR or --design inspect.json): whether a rule loosens a limit of the design cannot be checked without it"))?;
    if dir.exists() && !dir.is_dir() {
        return Err(CmdError::output(format!("{}: exists and is not a directory", dir.display())));
    }
    if dir.is_dir() && !force && std::fs::read_dir(&dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?.next().is_some() {
        return Err(CmdError::output(format!("{}: already exists and is not empty (use --force to replace its files {})", dir.display(), OWN_FILES.join(", "))));
    }

    let (sha, draft) = read_draft(&path)?;
    let (_, conf) = read_draft(&conf_path)?;
    let mut problems: Vec<String> = crate::jsonschema::validate(&serde_json::from_str(CONFIRMATION_SCHEMA).expect("valid schema"), &conf);
    if problems.is_empty() {
        if conf["format_version"].as_str().and_then(|v| v.split('.').next()) != Some("1") {
            problems.push(format!("confirmation format_version {} is not supported (1.x)", conf["format_version"]));
        }
        if conf["draft_sha256"].as_str() != Some(sha.as_str()) {
            problems.push(format!("the confirmation is for another draft (its draft_sha256 is {}, this draft's is {sha}); a draft that changed after it was confirmed must be confirmed again", conf["draft_sha256"]));
        }
        if conf["confirmed_by"]["name"].as_str().is_none_or(|n| n.trim().is_empty()) {
            problems.push("confirmed_by.name is empty: a person has to put their name on the decision".into());
        }
    }
    if !problems.is_empty() {
        return Err(CmdError::input(format!("{}: {}", conf_path.display(), problems.join("; "))));
    }

    let report = check::check(&draft, &sha, Some(&design.0), &draft_schema());
    if !report.valid {
        print_findings(&report);
        return Err(CmdError::input(format!("{}: the draft is not valid, nothing is applied", path.display())));
    }
    let list = |k: &str| -> HashSet<String> { conf[k].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
    let (accepted, relaxations, ack) = (list("accepted_rules"), list("accepted_relaxations"), list("acknowledged_not_executed"));
    let mut bad: Vec<String> = Vec::new();
    for v in &report.rules {
        match (accepted.contains(&v.id), ack.contains(&v.id)) {
            (true, true) => bad.push(format!("rule {} is both accepted and acknowledged as not executed", v.id)),
            (false, false) => bad.push(format!("rule {} is neither accepted nor acknowledged as not executed: every rule needs a decision", v.id)),
            (true, false) if !v.executable => bad.push(format!("rule {} cannot be executed ({}) and cannot be accepted", v.id, v.reason)),
            (true, false) if v.relaxes && !relaxations.contains(&v.id) => bad.push(format!("rule {} loosens a limit of the design: list it in accepted_relaxations to accept that explicitly", v.id)),
            _ => {}
        }
    }
    for id in accepted.iter().chain(ack.iter()).chain(relaxations.iter()) {
        if !report.rules.iter().any(|v| &v.id == id) {
            bad.push(format!("the confirmation names the rule '{id}', which the draft does not have"));
        }
    }
    for id in &relaxations {
        if report.rules.iter().any(|v| &v.id == id && !v.relaxes) || !accepted.contains(id) {
            bad.push(format!("{id} is listed in accepted_relaxations but is not an accepted rule that loosens a limit"));
        }
    }
    if !bad.is_empty() {
        return Err(CmdError::input(format!("{}: the confirmation does not match the draft: {}", conf_path.display(), bad.join("; "))));
    }

    let art = apply::build(&draft, &report, &accepted, &sha).map_err(CmdError::input)?;
    // from here on only writing: the directory, then its own files
    std::fs::create_dir_all(&dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?;
    for f in OWN_FILES {
        let target = dir.join(f);
        if target.exists() {
            if !force {
                return Err(CmdError::output(format!("{}: already exists (use --force)", target.display())));
            }
            std::fs::remove_file(&target).map_err(|e| CmdError::output(format!("{}: {e}", target.display())))?;
        }
    }
    let mut files: Vec<&str> = Vec::new();
    for (name, content) in [("pairs.txt", &art.pairs), ("tune.txt", &art.tune), ("rules.kicad_dru", &art.dru), ("route-args.txt", &art.route_args)] {
        if let Some(c) = content {
            write_atomically(&dir.join(name), c.as_bytes())?;
            files.push(name);
        }
    }
    let mut next = Vec::new();
    if art.route_args.is_some() {
        next.push("route with the written options: `fastroute route board.dsn -o routed.ses --pairs pairs.txt --tune tune.txt` (use the files that exist; route-args.txt lists them)".to_string());
    }
    if art.dru.is_some() {
        next.push("put rules.kicad_dru next to the board as <board>.kicad_dru; the KiCad plugin / route_cli.py carries its rules into the DSN".to_string());
    }
    next.extend(art.manual_steps.iter().cloned());
    next.push("check the result with the real checker: `fastroute ai-pack -i board.dsn -s routed.ses -o pack/ --kicad-pcb routed.kicad_pcb --require-drc`".to_string());
    let apply_report = json!({
        "format": "fastroute-rule-apply",
        "format_version": "1.0",
        "fastroute_version": env!("CARGO_PKG_VERSION"),
        "draft_sha256": sha,
        "confirmed_by": conf["confirmed_by"]["name"],
        "design_inspect_sha256": design.0.inspect_sha256,
        "executed": art.executed,
        "not_executed": art.not_executed,
        "relaxations_accepted": report.rules.iter().filter(|v| relaxations.contains(&v.id)).map(|v| &v.id).collect::<Vec<_>>(),
        "files": files,
        "manual_steps": art.manual_steps,
        "next_steps": next,
        "acceptance": "This report says what was written, not whether the board is right. The result is accepted only by the DRC of the routed board (ai-pack drc_status \"passed\" from a real kicad-cli report); no statement of an AI model or of this report replaces it.",
    });
    let mut text = serde_json::to_string_pretty(&apply_report).expect("serialises");
    text.push('\n');
    write_atomically(&dir.join("apply-report.json"), text.as_bytes())?;
    if !p.flag("quiet") {
        eprintln!(
            "{}: {} rules written ({}), {} not executed; nothing was routed. Next: see next_steps in apply-report.json",
            dir.display(), art.executed.len(), files.join(", "), art.not_executed.len()
        );
    }
    Ok(())
}
