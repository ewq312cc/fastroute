//! `fastroute constraints`: Hardware Constraint Packs (HCP) - check, compile, validate.
//!
//! ```text
//! constraints schema        the JSON Schema of a pack (or of the IR, or of a design snapshot)
//! constraints check PACK    the schema and the semantic checks of a pack (exit 3 if it is not valid)
//! constraints report PACK   a readable report of a pack (Chinese or English): what must not be
//!                           changed, what must stay, what may change, and the evidence
//! constraints compile PACK  the Constraint IR: deterministic, with the evidence of every rule
//! constraints validate      the IR (or a pack) against a design snapshot: exit 0 pass, 5 pass with
//!                           gaps, 1 violations, 3 invalid input
//! ```
//!
//! Everything is deterministic and offline: no AI model, no network, no vector index. The format
//! is described in `skills/hardware-constraint-design/references/hcp-format.md`.

use std::path::{Path, PathBuf};

use fr_constraint::diag::{Diagnostic, Severity};
use fr_constraint::hcp;
use fr_constraint::ir::{self, Ir};
use fr_constraint::jsonschema;
use fr_constraint::report::{self, Lang};
use fr_constraint::snapshot::{self, Snapshot};
use fr_constraint::{compile, validate};
use serde_json::{json, Value};

use crate::cmd::{self, check_output, opt, write_output, CmdError, Opt, Parsed, Spec};
use crate::load::sha256_hex;

const CHECK_OPTS: &[Opt] = &[
    opt("pack", None, Some("FILE"), "the pack, YAML or JSON (also the first positional argument)"),
    opt("json", None, None, "print the report as JSON on standard output"),
    opt("output", Some('o'), Some("FILE"), "write the JSON report here"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no diagnostics on standard error"),
];

const SCHEMA_OPTS: &[Opt] = &[
    opt("ir", None, None, "print the schema of the Constraint IR instead"),
    opt("snapshot", None, None, "print the schema of a design snapshot instead"),
];
const REPORT_OPTS: &[Opt] = &[
    opt("pack", None, Some("FILE"), "the pack, YAML or JSON (also the first positional argument)"),
    opt("output", Some('o'), Some("FILE"), "the report (Markdown) to write ('-' = standard output)"),
    opt("lang", None, Some("zh|en"), "language of the report (default: the pack's language, else zh)"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no diagnostics on standard error"),
];
const COMPILE_OPTS: &[Opt] = &[
    opt("pack", None, Some("FILE"), "the pack, YAML or JSON (also the first positional argument)"),
    opt("output", Some('o'), Some("FILE"), "the Constraint IR to write ('-' = standard output)"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];
const VALIDATE_OPTS: &[Opt] = &[
    opt("ir", None, Some("FILE"), "the compiled Constraint IR"),
    opt("pack", None, Some("FILE"), "a pack, compiled on the fly (instead of --ir)"),
    opt("design", None, Some("FILE"), "the design snapshot (JSON)"),
    opt("json", None, None, "print the validation as JSON on standard output"),
    opt("output", Some('o'), Some("FILE"), "write the JSON validation here"),
    opt("report", None, Some("FILE"), "write a readable report (Markdown) here"),
    opt("lang", None, Some("zh|en"), "language of the report (default zh)"),
    opt("force", None, None, "replace existing output files"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];

const fn spec(name: &'static str, about: &'static str, usage: &'static str, opts: &'static [Opt], footer: &'static str) -> Spec {
    Spec { name, about, usage, opts, passthrough: &[], footer }
}

const SCHEMA_SPEC: Spec = spec("constraints schema", "print the JSON Schema of a hardware constraint pack (or IR, or design snapshot)", "constraints schema [--ir | --snapshot]", SCHEMA_OPTS, "The same files are in skills/hardware-constraint-design/schemas/; the commands enforce them.");
const REPORT_SPEC: Spec = spec(
    "constraints report",
    "write a readable report of a pack: what must not be changed, what must stay, what may change, and the evidence",
    "constraints report PACK --output report.md [--lang zh|en]",
    REPORT_OPTS,
    "Sections, always in this order: 1 cannot be changed, 2 must stay, 3 may be configured, 4 where an AI may edit (the\nwhitelist), 5 high-speed / mechanical / routing, 6 conflicts and gaps, 7 sources. Every item cites document, version, page /\nsection and the quoted words; derivations, observations and pending notes are labelled. A pack with errors has no report (exit 3).",
);
const COMPILE_SPEC: Spec = spec(
    "constraints compile",
    "compile a pack to the Constraint IR (deterministic; evidence kept; unsupported rules marked)",
    "constraints compile PACK --output ir.json",
    COMPILE_OPTS,
    "The same pack and program version give the same bytes. A pack with errors is not compiled (exit 3). Per rule the IR says\nwhether the validator and the fastroute adapter can execute it; nothing is dropped or approximated silently.",
);
const VALIDATE_SPEC: Spec = spec(
    "constraints validate",
    "check a design snapshot against a Constraint IR: required blocks, reserved pins, pin functions, fixed nets, keep-outs",
    "constraints validate (--ir ir.json | --pack pack.yaml) --design design.json [--output v.json] [--report v.md]",
    VALIDATE_OPTS,
    "Exit codes: 0 pass and every rule checked, 5 pass but some rules are unchecked / unsupported / unknown / in conflict,\n1 a rule is violated, 3 invalid input, 4 output refused. Deterministic and offline: no AI model, network or vector index.",
);


const CHECK_SPEC: Spec = spec(
    "constraints check",
    "check a pack: structure, references, evidence, units, whitelist of what is editable",
    "constraints check PACK [--json] [--output report.json]",
    CHECK_OPTS,
    "Exit 0 if the pack is valid (warnings are listed), 3 if it is not. Needs no network, AI model or vector index.\nDiagnostic codes: skills/hardware-constraint-design/references/hcp-format.md.",
);

const SUBCOMMANDS: &str = "\
commands:
  schema              print the JSON Schema of a pack (or the IR, or a design snapshot)
  check               check a pack (exit 3 if it is not valid)
  report              a readable report of a pack (zh or en)
  compile             compile a pack to the Constraint IR
  validate            check a design snapshot against the IR

Run `fastroute constraints <command> --help` for the options. Format: skills/hardware-constraint-design/references/hcp-format.md.
";

pub fn help_text() -> String {
    format!("fastroute constraints - hardware constraint packs (HCP): deterministic, offline checks\n\nusage: fastroute constraints <command> [options]\n\n{SUBCOMMANDS}")
}

pub fn help_for(sub: Option<&str>) -> String {
    match sub {
        Some("schema") => SCHEMA_SPEC.help_text(),
        Some("check") => CHECK_SPEC.help_text(),
        Some("report") => REPORT_SPEC.help_text(),
        Some("compile") => COMPILE_SPEC.help_text(),
        Some("validate") => VALIDATE_SPEC.help_text(),
        _ => help_text(),
    }
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
        "check" => cmd_check(rest),
        "report" => cmd_report(rest),
        "compile" => cmd_compile(rest),
        "validate" => cmd_validate(rest),
        other => Err(CmdError::usage(format!("unknown command 'constraints {other}' (schema, check, report, compile, validate)"))),
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

fn cmd_schema(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&SCHEMA_SPEC, args)? else { return Ok(()) };
    if p.flag("ir") && p.flag("snapshot") {
        return Err(CmdError::usage("--ir and --snapshot contradict each other"));
    }
    print!("{}", if p.flag("ir") { ir::IR_SCHEMA_TEXT } else if p.flag("snapshot") { snapshot::SNAPSHOT_SCHEMA_TEXT } else { hcp::SCHEMA_TEXT });
    Ok(())
}

/// The file named by `--pack` or the one positional argument.
fn pack_path(p: &Parsed) -> Result<PathBuf, CmdError> {
    match (p.path("pack"), p.positional.as_slice()) {
        (Some(f), []) => Ok(f),
        (None, [one]) => Ok(PathBuf::from(one)),
        (Some(_), _) => Err(CmdError::usage(format!("unexpected argument '{}': the pack is given with --pack", p.positional[0]))),
        (None, []) => Err(CmdError::usage("no pack file")),
        (None, _) => Err(CmdError::usage("more than one pack given")),
    }
}

/// Reads and parses a pack: (path, bytes' SHA-256, document).
pub fn read_pack(path: &Path) -> Result<(String, Value), CmdError> {
    let bytes = std::fs::read(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    let text = String::from_utf8(bytes.clone()).map_err(|_| CmdError::input(format!("{}: not UTF-8 text", path.display())))?;
    let doc = hcp::parse(&text).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    Ok((sha256_hex(&bytes), doc))
}

pub fn print_diagnostics(ds: &[Diagnostic]) {
    for d in ds {
        let sev = match d.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        match &d.object {
            Some(o) => eprintln!("{sev}[{}] {} ({o}): {}", d.code, d.path, d.message),
            None => eprintln!("{sev}[{}] {}: {}", d.code, d.path, d.message),
        }
    }
}

fn cmd_check(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&CHECK_SPEC, args)? else { return Ok(()) };
    let path = pack_path(&p)?;
    let output = p.path("output");
    if let Some(o) = &output {
        check_output(o, &[&path], p.flag("force"))?;
    }
    let (sha, doc) = read_pack(&path)?;
    let diagnostics = hcp::check(&doc);
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    let warnings = diagnostics.len() - errors;
    let report = json!({
        "format": "fastroute-hcp-check",
        "format_version": "1.0",
        "fastroute_version": env!("CARGO_PKG_VERSION"),
        "input": {"path": path.display().to_string(), "sha256": sha},
        "valid": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "diagnostics": diagnostics,
    });
    let mut text = serde_json::to_string_pretty(&report).expect("serialises");
    text.push('\n');
    if let Some(o) = &output {
        write_output(o, text.as_bytes(), &[&path], p.flag("force"))?;
    }
    if p.flag("json") {
        print!("{text}");
    }
    if !p.flag("quiet") {
        print_diagnostics(&diagnostics);
        eprintln!("{}: {} ({} errors, {} warnings)", path.display(), if errors == 0 { "valid" } else { "NOT valid" }, errors, warnings);
    }
    if errors == 0 {
        Ok(())
    } else {
        Err(CmdError::input(format!("{}: the pack is not valid ({errors} errors)", path.display())))
    }
}

fn cmd_compile(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&COMPILE_SPEC, args)? else { return Ok(()) };
    let path = pack_path(&p)?;
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output (--output ir.json, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    if !to_stdout {
        check_output(&output, &[&path], p.flag("force"))?;
    }
    let (_, doc) = read_pack(&path)?;
    let ir = match compile::compile(&doc) {
        Ok(ir) => ir,
        Err(ds) => {
            print_diagnostics(&ds);
            let n = ds.iter().filter(|d| d.severity == Severity::Error).count();
            return Err(CmdError::input(format!("{}: the pack is not valid ({n} errors), nothing was compiled", path.display())));
        }
    };
    let text = ir.to_json_text();
    if to_stdout {
        print!("{text}");
    } else {
        write_output(&output, text.as_bytes(), &[&path], p.flag("force"))?;
    }
    if !p.flag("quiet") {
        print_diagnostics(&ir.diagnostics);
        let n = |s: &str| ir.rules.iter().filter(|r| r.status == s).count();
        eprintln!(
            "{}: {} rules ({} executable, {} unsupported, {} unknown, {} conflict); IR sha256 {}",
            if to_stdout { "IR".to_string() } else { output.display().to_string() },
            ir.rules.len(),
            n("executable"),
            n("unsupported"),
            n("unknown"),
            n("conflict"),
            ir.integrity.sha256
        );
    }
    Ok(())
}

/// A JSON file checked against one of the published schemas.
fn read_json_checked(path: &Path, schema_text: &str, what: &str) -> Result<(String, Value), CmdError> {
    let bytes = std::fs::read(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| CmdError::input(format!("{}: not valid JSON: {e}", path.display())))?;
    let schema: Value = serde_json::from_str(schema_text).expect("a published schema is valid JSON");
    let errors = jsonschema::validate(&schema, &v);
    if !errors.is_empty() {
        for e in errors.iter().take(20) {
            eprintln!("error[schema] {}: {}", e.path, e.message);
        }
        return Err(CmdError::input(format!("{}: not a valid {what} ({} problems)", path.display(), errors.len())));
    }
    Ok((sha256_hex(&bytes), v))
}

fn cmd_validate(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&VALIDATE_SPEC, args)? else { return Ok(()) };
    let lang = match p.value("lang") {
        None => Lang::Zh,
        Some(l) => Lang::parse(l).ok_or_else(|| CmdError::usage(format!("unknown language '{l}' (zh or en)")))?,
    };
    let design_path = p.path("design").ok_or_else(|| CmdError::usage("no design (--design design.json)"))?;
    let (ir_path, pack_path_opt) = (p.path("ir"), p.path("pack"));
    if ir_path.is_some() == pack_path_opt.is_some() {
        return Err(CmdError::usage("give either --ir (a compiled IR) or --pack (compiled on the fly)"));
    }
    let outputs: Vec<(PathBuf, &str)> = [(p.path("output"), "output"), (p.path("report"), "report")].into_iter().filter_map(|(o, n)| o.map(|o| (o, n))).collect();
    let mut inputs: Vec<&Path> = vec![&design_path];
    inputs.extend(ir_path.as_deref());
    inputs.extend(pack_path_opt.as_deref());
    for (o, _) in &outputs {
        check_output(o, &inputs, p.flag("force"))?;
    }
    // inputs, in the order usage -> input
    let ir: Ir = match (&ir_path, &pack_path_opt) {
        (Some(f), _) => {
            let (_, v) = read_json_checked(f, ir::IR_SCHEMA_TEXT, "Constraint IR")?;
            serde_json::from_value(v).map_err(|e| CmdError::input(format!("{}: {e}", f.display())))?
        }
        (_, Some(f)) => {
            let (_, doc) = read_pack(f)?;
            match compile::compile(&doc) {
                Ok(ir) => ir,
                Err(ds) => {
                    print_diagnostics(&ds);
                    return Err(CmdError::input(format!("{}: the pack is not valid, nothing to validate against", f.display())));
                }
            }
        }
        _ => unreachable!("checked above"),
    };
    let (design_sha, design_json) = read_json_checked(&design_path, snapshot::SNAPSHOT_SCHEMA_TEXT, "design snapshot")?;
    let snap: Snapshot = serde_json::from_value(design_json).map_err(|e| CmdError::input(format!("{}: {e}", design_path.display())))?;
    let v = validate::validate(&ir, &snap, &snap.project.name, Some(&design_sha)).map_err(CmdError::input)?;
    let mut json_text = serde_json::to_string_pretty(&v).expect("serialises");
    json_text.push('\n');
    for (o, kind) in &outputs {
        let bytes = if *kind == "output" { json_text.clone() } else { report::validation_text(&v, lang) };
        write_output(o, bytes.as_bytes(), &inputs, p.flag("force"))?;
    }
    if p.flag("json") {
        print!("{json_text}");
    }
    if !p.flag("quiet") {
        for f in v.findings.iter().filter(|f| f.verdict == validate::Verdict::Fail) {
            eprintln!(
                "{}[{}] {}{}: {}",
                if f.severity == "error" { "error" } else { "warning" },
                f.rule_id,
                f.kind,
                f.object.as_ref().map(|o| format!(" ({o})")).unwrap_or_default(),
                f.message
            );
        }
        let s = &v.summary;
        eprintln!(
            "{}: {} - {} rules: {} passed, {} failed, {} not applicable, {} not checked, {} unsupported, {} unknown, {} in conflict",
            design_path.display(),
            if v.result == "pass" { "pass" } else { "FAIL" },
            s.rules, s.passed, s.failed, s.not_applicable, s.unchecked, s.unsupported, s.unknown, s.conflict
        );
    }
    match (v.result, v.complete) {
        ("fail", _) => Err(CmdError::new(cmd::exit::FAILED, format!("{}: {} rule(s) violated", design_path.display(), v.summary.failed))),
        (_, false) => Err(CmdError::new(cmd::exit::PARTIAL, "no rule is violated, but some rules are unchecked, unsupported, unknown or in conflict (see the report)")),
        _ => Ok(()),
    }
}

fn cmd_report(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&REPORT_SPEC, args)? else { return Ok(()) };
    let path = pack_path(&p)?;
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output (--output report.md, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    let explicit_lang = match p.value("lang") {
        None => None,
        Some(l) => Some(Lang::parse(l).ok_or_else(|| CmdError::usage(format!("unknown language '{l}' (zh or en)")))?),
    };
    if !to_stdout {
        check_output(&output, &[&path], p.flag("force"))?;
    }
    let (_, doc) = read_pack(&path)?;
    let diagnostics = hcp::check(&doc);
    let errors = diagnostics.iter().filter(|d| d.severity == Severity::Error).count();
    if errors > 0 {
        print_diagnostics(&diagnostics);
        return Err(CmdError::input(format!("{}: the pack is not valid ({errors} errors): there is no report of an invalid pack", path.display())));
    }
    let lang = explicit_lang.unwrap_or(if doc["pack"]["language"] == "en" { Lang::En } else { Lang::Zh });
    let text = fr_constraint::pack_report::pack_report(&doc, lang);
    if to_stdout {
        print!("{text}");
    } else {
        write_output(&output, text.as_bytes(), &[&path], p.flag("force"))?;
    }
    if !p.flag("quiet") {
        print_diagnostics(&diagnostics);
    }
    Ok(())
}
