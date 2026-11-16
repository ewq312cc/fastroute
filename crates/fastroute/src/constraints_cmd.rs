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
//! constraints edit-request  the request for an AI that may edit a fixed base design: the whitelist
//! constraints check-edit    an AI edit draft against the whitelist and the rules: exit 0 accepted,
//!                           5 needs review / stopped / not fully checked, 1 rejected, 3 invalid input
//! ```
//!
//! Everything is deterministic and offline: no AI model, no network, no vector index. The format
//! is described in `skills/hardware-constraint-design/references/hcp-format.md`.

use std::path::{Path, PathBuf};

use fr_constraint::diag::{Diagnostic, Severity};
use fr_constraint::edit;
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
    opt("edit-draft", None, None, "print the schema of an AI edit draft instead"),
    opt("edit-request", None, None, "print the schema of an edit request instead"),
];
const EDIT_REQUEST_OPTS: &[Opt] = &[
    opt("ir", None, Some("FILE"), "the compiled Constraint IR"),
    opt("pack", None, Some("FILE"), "a pack, compiled on the fly (instead of --ir)"),
    opt("requirement", None, Some("TEXT"), "the project requirement, in words"),
    opt("requirement-file", None, Some("FILE"), "read the requirement from this file"),
    opt("output", Some('o'), Some("FILE"), "the request (JSON) to write ('-' = standard output)"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];
const CHECK_EDIT_OPTS: &[Opt] = &[
    opt("draft", None, Some("FILE"), "the AI edit draft, JSON or YAML (also the first positional argument)"),
    opt("ir", None, Some("FILE"), "the compiled Constraint IR"),
    opt("pack", None, Some("FILE"), "a pack, compiled on the fly (instead of --ir)"),
    opt("design", None, Some("FILE"), "the base design as a snapshot (JSON)"),
    opt("kicad-pcb", None, Some("FILE"), "the base design as a KiCad board, read-only"),
    opt("base-dir", None, Some("DIR"), "the working copy of the base design: the files the pack binds are compared by SHA-256"),
    opt("json", None, None, "print the result as JSON on standard output"),
    opt("output", Some('o'), Some("FILE"), "write the JSON result here"),
    opt("report", None, Some("FILE"), "write the readable difference report (Markdown) here"),
    opt("lang", None, Some("zh|en"), "language of the report (default zh)"),
    opt("force", None, None, "replace existing output files"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
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
const EXTRACT_OPTS: &[Opt] = &[
    opt("board", None, Some("FILE"), "the KiCad board, .kicad_pcb (also the first positional argument)"),
    opt("output", Some('o'), Some("FILE"), "the design snapshot (JSON) to write ('-' = standard output)"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];
const EXPORT_OPTS: &[Opt] = &[
    opt("ir", None, Some("FILE"), "the compiled Constraint IR"),
    opt("pack", None, Some("FILE"), "a pack, compiled on the fly (instead of --ir)"),
    opt("design", None, Some("FILE"), "a design snapshot (JSON): resolves net patterns"),
    opt("kicad-pcb", None, Some("FILE"), "a KiCad board instead of a snapshot: resolves net patterns"),
    opt("output-dir", Some('o'), Some("DIR"), "directory for pairs.txt, tune.txt, rules.kicad_dru, route-args.txt and export-report.json"),
    opt("force", None, None, "replace those files in an existing directory (nothing else is touched)"),
    opt("quiet", Some('q'), None, "no summary on standard error"),
];
const VALIDATE_OPTS: &[Opt] = &[
    opt("ir", None, Some("FILE"), "the compiled Constraint IR"),
    opt("pack", None, Some("FILE"), "a pack, compiled on the fly (instead of --ir)"),
    opt("design", None, Some("FILE"), "the design snapshot (JSON)"),
    opt("kicad-pcb", None, Some("FILE"), "a KiCad board, read-only, instead of a snapshot"),
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

const SCHEMA_SPEC: Spec = spec("constraints schema", "print the JSON Schema of a hardware constraint pack (or IR, design snapshot, edit draft or edit request)", "constraints schema [--ir | --snapshot | --edit-draft | --edit-request]", SCHEMA_OPTS, "The same files are in skills/hardware-constraint-design/schemas/; the commands enforce them.");
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
    "check a design (snapshot or KiCad board) against a Constraint IR: required blocks, reserved pins, pin functions, fixed nets, keep-outs",
    "constraints validate (--ir ir.json | --pack pack.yaml) (--design design.json | --kicad-pcb board.kicad_pcb) [--output v.json] [--report v.md]",
    VALIDATE_OPTS,
    "Exit codes: 0 pass and every rule checked, 5 pass but some rules are unchecked / unsupported / unknown / in conflict,\n1 a rule is violated, 3 invalid input, 4 output refused. Deterministic and offline: no AI model, network or vector index.\nA KiCad board is only read, never written. This is not KiCad's DRC: the two results are separate (fastroute ai-pack).",
);
const EXTRACT_SPEC: Spec = spec(
    "constraints extract-kicad",
    "read a KiCad board (read-only) into a design snapshot: components, pads and nets, tracks, vias, zones, outline, mounting holes",
    "constraints extract-kicad BOARD.kicad_pcb --output design.json",
    EXTRACT_OPTS,
    "Millimetres, x to the right, y down (KiCad's frame), pad positions absolute. Arcs become two chords; an outline that cannot be\nchained into one closed polygon becomes the bounding box of the Edge.Cuts items. The board is never modified.",
);
const EXPORT_SPEC: Spec = spec(
    "constraints export-fastroute",
    "write the rules fastroute can execute (pairs, length matching, clearance, track width) and report what it cannot",
    "constraints export-fastroute (--ir ir.json | --pack pack.yaml) --output-dir DIR [--design design.json | --kicad-pcb board.kicad_pcb]",
    EXPORT_OPTS,
    "fastroute reads only the written files. Rules that are unknown or in conflict are never exported; rules fastroute cannot\nexecute (keep-outs, impedance, maximum length, ...) are listed in export-report.json with the reason, never dropped quietly.",
);


const EDIT_REQUEST_SPEC: Spec = spec(
    "constraints edit-request",
    "write the request for a semantic generation run on a fixed base design: pack, base, requirement and the whitelist",
    "constraints edit-request (--ir ir.json | --pack pack.yaml) --requirement TEXT --output request.json",
    EDIT_REQUEST_OPTS,
    "The whitelist (editable regions, allowed blocks, pin pools without protected pins, allowed operations) is computed from the pack:\nsame pack, same request. It refers to the pack and the base by SHA-256, so that a draft made for another version is sent back.\nA pack without base_design has nothing editable and gets no request.",
);
const CHECK_EDIT_SPEC: Spec = spec(
    "constraints check-edit",
    "check an AI edit draft: only whitelisted regions, blocks, pins and operations; fixed objects untouched; the pack's rules still met",
    "constraints check-edit DRAFT (--ir ir.json | --pack pack.yaml) [--design design.json | --kicad-pcb board.kicad_pcb] [--base-dir DIR] [--report diff.md]",
    CHECK_EDIT_OPTS,
    "Exit codes: 0 accepted and fully checked, 5 accepted with parts unchecked / the pack or the base design changed (review) / the\ngenerator stopped, 1 rejected (a change is outside what is authorised, or breaks a rule), 3 invalid input, 4 output refused.\nThe base design is only read: the draft is never applied to it, and an output inside --base-dir or onto a bound file is refused.\nSearch results (keyword or vector) in the draft are information: they change no whitelist. Offline, deterministic.",
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
  validate            check a design (snapshot or KiCad board) against the IR
  extract-kicad       read a KiCad board (read-only) into a design snapshot
  export-fastroute    write the rules fastroute can execute, and report what it cannot
  edit-request        the request for an AI editing a fixed base design (the whitelist)
  check-edit          check an AI edit draft against the whitelist and the rules

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
        Some("extract-kicad") => EXTRACT_SPEC.help_text(),
        Some("export-fastroute") => EXPORT_SPEC.help_text(),
        Some("edit-request") => EDIT_REQUEST_SPEC.help_text(),
        Some("check-edit") => CHECK_EDIT_SPEC.help_text(),
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
        "extract-kicad" => cmd_extract_kicad(rest),
        "export-fastroute" => cmd_export(rest),
        "edit-request" => cmd_edit_request(rest),
        "check-edit" => cmd_check_edit(rest),
        other => Err(CmdError::usage(format!("unknown command 'constraints {other}' (schema, check, report, compile, validate, extract-kicad, export-fastroute, edit-request, check-edit)"))),
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
    let chosen: Vec<&str> = ["ir", "snapshot", "edit-draft", "edit-request"].into_iter().filter(|f| p.flag(f)).collect();
    if chosen.len() > 1 {
        return Err(CmdError::usage(format!("--{} and --{} contradict each other", chosen[0], chosen[1])));
    }
    print!(
        "{}",
        match chosen.first().copied() {
            Some("ir") => ir::IR_SCHEMA_TEXT,
            Some("snapshot") => snapshot::SNAPSHOT_SCHEMA_TEXT,
            Some("edit-draft") => edit::DRAFT_SCHEMA_TEXT,
            Some("edit-request") => edit::REQUEST_SCHEMA_TEXT,
            _ => hcp::SCHEMA_TEXT,
        }
    );
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

/// A design: a snapshot (JSON, checked against its schema) or a KiCad board (read, never written).
/// Returns the digest of the file and the snapshot.
fn read_design(path: &Path, kicad: bool) -> Result<(String, Snapshot), CmdError> {
    if kicad {
        let bytes = std::fs::read(path).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
        let text = String::from_utf8(bytes.clone()).map_err(|_| CmdError::input(format!("{}: not UTF-8 text", path.display())))?;
        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let snap = fr_constraint::kicad::read_pcb(&text, &name).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
        return Ok((sha256_hex(&bytes), snap));
    }
    let (sha, v) = read_json_checked(path, snapshot::SNAPSHOT_SCHEMA_TEXT, "design snapshot")?;
    let snap: Snapshot = serde_json::from_value(v).map_err(|e| CmdError::input(format!("{}: {e}", path.display())))?;
    Ok((sha, snap))
}

/// The Constraint IR of `--ir` or `--pack` (exactly one of them).
fn read_ir(p: &Parsed) -> Result<(Ir, PathBuf), CmdError> {
    match (p.path("ir"), p.path("pack")) {
        (Some(f), None) => {
            let (_, v) = read_json_checked(&f, ir::IR_SCHEMA_TEXT, "Constraint IR")?;
            Ok((serde_json::from_value(v).map_err(|e| CmdError::input(format!("{}: {e}", f.display())))?, f))
        }
        (None, Some(f)) => {
            let (_, doc) = read_pack(&f)?;
            match compile::compile(&doc) {
                Ok(ir) => Ok((ir, f)),
                Err(ds) => {
                    print_diagnostics(&ds);
                    Err(CmdError::input(format!("{}: the pack is not valid, nothing to work from", f.display())))
                }
            }
        }
        _ => Err(CmdError::usage("give either --ir (a compiled IR) or --pack (compiled on the fly)")),
    }
}

fn cmd_extract_kicad(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&EXTRACT_SPEC, args)? else { return Ok(()) };
    let board = match (p.path("board"), p.positional.as_slice()) {
        (Some(b), []) => b,
        (None, [one]) => PathBuf::from(one),
        (Some(_), _) => return Err(CmdError::usage(format!("unexpected argument '{}': the board is given with --board", p.positional[0]))),
        (None, []) => return Err(CmdError::usage("no board (a .kicad_pcb file)")),
        (None, _) => return Err(CmdError::usage("more than one board given")),
    };
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output (--output design.json, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    if !to_stdout {
        check_output(&output, &[&board], p.flag("force"))?;
    }
    let (_, snap) = read_design(&board, true)?;
    let mut text = serde_json::to_string_pretty(&snap).expect("serialises");
    text.push('\n');
    if to_stdout {
        print!("{text}");
    } else {
        write_output(&output, text.as_bytes(), &[&board], p.flag("force"))?;
    }
    if !p.flag("quiet") {
        eprintln!(
            "{}: {} components, {} pads, {} tracks, {} vias, {} zones, {} mounting holes, outline of {} points",
            board.display(),
            snap.components.len(),
            snap.pins.len(),
            snap.tracks.len(),
            snap.vias.len(),
            snap.zones.len(),
            snap.mount_holes.len(),
            snap.outline.len()
        );
    }
    Ok(())
}

fn cmd_export(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&EXPORT_SPEC, args)? else { return Ok(()) };
    let dir = p.path("output-dir").ok_or_else(|| CmdError::usage("no output directory (--output-dir DIR)"))?;
    let (design_opt, kicad_opt) = (p.path("design"), p.path("kicad-pcb"));
    if design_opt.is_some() && kicad_opt.is_some() {
        return Err(CmdError::usage("give either --design or --kicad-pcb, not both"));
    }
    let (ir, ir_source) = read_ir(&p)?;
    let design = match (&design_opt, &kicad_opt) {
        (Some(d), None) => Some(read_design(d, false)?.1),
        (None, Some(k)) => Some(read_design(k, true)?.1),
        _ => None,
    };
    let export = fr_constraint::export::export_fastroute(&ir, design.as_ref()).map_err(CmdError::input)?;
    let files: Vec<(&str, Option<String>)> = vec![
        ("pairs.txt", export.pairs.clone()),
        ("tune.txt", export.tune.clone()),
        ("rules.kicad_dru", export.dru.clone()),
        ("route-args.txt", export.route_args.clone()),
        ("export-report.json", Some(format!("{}\n", serde_json::to_string_pretty(&export.report).expect("serialises")))),
    ];
    let inputs: Vec<&Path> = [Some(ir_source.as_path()), design_opt.as_deref(), kicad_opt.as_deref()].into_iter().flatten().collect();
    // nothing is written if any of the files would be refused (a directory that does not exist yet
    // cannot hold anything in the way)
    if dir.exists() {
        for (name, content) in &files {
            if content.is_some() {
                check_output(&dir.join(name), &inputs, p.flag("force"))?;
            }
        }
    } else {
        std::fs::create_dir_all(&dir).map_err(|e| CmdError::output(format!("{}: {e}", dir.display())))?;
    }
    for (name, content) in &files {
        if let Some(c) = content {
            write_output(&dir.join(name), c.as_bytes(), &inputs, p.flag("force"))?;
        }
    }
    if !p.flag("quiet") {
        let n = |k: &str| export.report[k].as_array().map(Vec::len).unwrap_or(0);
        eprintln!(
            "{}: {} rule(s) written ({}), {} not exported, {} parameter(s) not handed over; the rest is checked by the validator, not the router",
            dir.display(),
            n("exported"),
            files.iter().filter(|(_, c)| c.is_some()).map(|(f, _)| *f).filter(|f| *f != "export-report.json").collect::<Vec<_>>().join(", "),
            n("not_exported"),
            n("unmapped_params")
        );
    }
    Ok(())
}

fn cmd_validate(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&VALIDATE_SPEC, args)? else { return Ok(()) };
    let lang = match p.value("lang") {
        None => Lang::Zh,
        Some(l) => Lang::parse(l).ok_or_else(|| CmdError::usage(format!("unknown language '{l}' (zh or en)")))?,
    };
    let design_path = match (p.path("design"), p.path("kicad-pcb")) {
        (Some(_), Some(_)) => return Err(CmdError::usage("give either --design (a snapshot) or --kicad-pcb (a board), not both")),
        (Some(d), None) | (None, Some(d)) => d,
        (None, None) => return Err(CmdError::usage("no design (--design design.json or --kicad-pcb board.kicad_pcb)")),
    };
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
    let (design_sha, snap) = read_design(&design_path, p.path("kicad-pcb").is_some())?;
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

fn cmd_edit_request(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&EDIT_REQUEST_SPEC, args)? else { return Ok(()) };
    let output = p.path("output").ok_or_else(|| CmdError::usage("no output (--output request.json, or - for standard output)"))?;
    let to_stdout = output == Path::new("-");
    let requirement = match (p.value("requirement"), p.path("requirement-file")) {
        (Some(_), Some(_)) => return Err(CmdError::usage("give either --requirement or --requirement-file, not both")),
        (Some(t), None) => t.to_string(),
        (None, Some(f)) => std::fs::read_to_string(&f).map_err(|e| CmdError::input(format!("{}: {e}", f.display())))?,
        (None, None) => String::new(),
    };
    let mut inputs: Vec<PathBuf> = vec![];
    inputs.extend(p.path("ir"));
    inputs.extend(p.path("pack"));
    let refs: Vec<&Path> = inputs.iter().map(PathBuf::as_path).collect();
    if !to_stdout {
        check_output(&output, &refs, p.flag("force"))?;
    }
    let (ir, _) = read_ir(&p)?;
    let request = edit::edit_request(&ir, requirement.trim()).map_err(CmdError::input)?;
    let mut text = serde_json::to_string_pretty(&request).expect("serialises");
    text.push('\n');
    if to_stdout {
        print!("{text}");
    } else {
        write_output(&output, text.as_bytes(), &refs, p.flag("force"))?;
    }
    if !p.flag("quiet") {
        let regions = request["authorization"]["regions"].as_array().map(Vec::len).unwrap_or(0);
        eprintln!(
            "{}: request for pack {} {} and base {}: {} editable region(s), {} protected pin(s)",
            if to_stdout { "request".to_string() } else { output.display().to_string() },
            ir.pack.id,
            ir.pack.version,
            request["base"]["id"].as_str().unwrap_or(""),
            regions,
            request["authorization"]["protected_pins"].as_array().map(Vec::len).unwrap_or(0)
        );
    }
    Ok(())
}

/// The files the pack binds, as they are in `dir`: a path that leaves the directory is refused.
fn base_files(ir: &Ir, dir: &Path) -> Result<(Vec<edit::BaseFile>, Vec<PathBuf>), CmdError> {
    let mut files = Vec::new();
    let mut paths = Vec::new();
    let Some(auth) = edit::authorization(ir) else { return Ok((files, paths)) };
    for f in auth.binding.get("files").and_then(Value::as_array).into_iter().flatten() {
        let Some(rel) = f.get("path").and_then(Value::as_str) else { continue };
        let path = Path::new(rel);
        if path.is_absolute() || path.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(CmdError::input(format!("the pack binds the file '{rel}', which is outside the base directory: refused")));
        }
        let full = dir.join(path);
        let sha = std::fs::read(&full).ok().map(|b| sha256_hex(&b));
        files.push(edit::BaseFile { path: rel.to_string(), sha256: sha });
        paths.push(full);
    }
    Ok((files, paths))
}

fn cmd_check_edit(args: &[String]) -> Result<(), CmdError> {
    let Some(p) = parse(&CHECK_EDIT_SPEC, args)? else { return Ok(()) };
    let lang = match p.value("lang") {
        None => Lang::Zh,
        Some(l) => Lang::parse(l).ok_or_else(|| CmdError::usage(format!("unknown language '{l}' (zh or en)")))?,
    };
    let draft_path = match (p.path("draft"), p.positional.as_slice()) {
        (Some(f), []) => f,
        (None, [one]) => PathBuf::from(one),
        (Some(_), _) => return Err(CmdError::usage(format!("unexpected argument '{}': the draft is given with --draft", p.positional[0]))),
        (None, []) => return Err(CmdError::usage("no draft file")),
        (None, _) => return Err(CmdError::usage("more than one draft given")),
    };
    let design_path = match (p.path("design"), p.path("kicad-pcb")) {
        (Some(_), Some(_)) => return Err(CmdError::usage("give either --design (a snapshot) or --kicad-pcb (a board), not both")),
        (d, k) => d.or(k),
    };
    let (ir_path, pack_path_opt) = (p.path("ir"), p.path("pack"));
    if ir_path.is_some() == pack_path_opt.is_some() {
        return Err(CmdError::usage("give either --ir (a compiled IR) or --pack (compiled on the fly)"));
    }
    let base_dir = p.path("base-dir");
    if let Some(d) = &base_dir {
        if !d.is_dir() {
            return Err(CmdError::input(format!("{}: not a directory", d.display())));
        }
    }
    let (ir, _) = read_ir(&p)?;
    let (bfiles, bpaths) = match &base_dir {
        Some(d) => {
            let (f, paths) = base_files(&ir, d)?;
            (Some(f), paths)
        }
        None => (None, vec![]),
    };
    // outputs: never an input, never a file of the base design, never inside the base directory
    let outputs: Vec<(PathBuf, &str)> = [(p.path("output"), "output"), (p.path("report"), "report")].into_iter().filter_map(|(o, n)| o.map(|o| (o, n))).collect();
    let mut inputs: Vec<&Path> = vec![&draft_path];
    inputs.extend(design_path.as_deref());
    inputs.extend(ir_path.as_deref());
    inputs.extend(pack_path_opt.as_deref());
    inputs.extend(bpaths.iter().map(PathBuf::as_path));
    for (o, _) in &outputs {
        check_output(o, &inputs, p.flag("force"))?;
        if let Some(d) = &base_dir {
            let parent = o.parent().filter(|x| !x.as_os_str().is_empty()).unwrap_or(Path::new("."));
            if let (Ok(par), Ok(dir)) = (parent.canonicalize(), d.canonicalize()) {
                if par.starts_with(&dir) {
                    return Err(CmdError::output(format!("{}: inside the base directory {}: the base design is never written; put the report elsewhere", o.display(), d.display())));
                }
            }
        }
    }
    let draft_text = std::fs::read_to_string(&draft_path).map_err(|e| CmdError::input(format!("{}: {e}", draft_path.display())))?;
    let draft = hcp::parse(&draft_text).map_err(|e| CmdError::input(format!("{}: {e}", draft_path.display())))?;
    let design = match &design_path {
        Some(d) => {
            let (_, mut snap) = read_design(d, p.path("kicad-pcb").is_some())?;
            if p.path("kicad-pcb").is_some() {
                snap.project.source_sha256 = Some(sha256_hex(&std::fs::read(d).map_err(|e| CmdError::input(format!("{}: {e}", d.display())))?));
            }
            Some(snap)
        }
        None => None,
    };
    let check = match edit::check_edit(&ir, &draft, design.as_ref(), bfiles.as_deref()) {
        Ok(c) => c,
        Err(ds) => {
            print_diagnostics(&ds);
            return Err(CmdError::input(format!("{}: not a valid edit draft ({} problems)", draft_path.display(), ds.len())));
        }
    };
    let mut json_text = serde_json::to_string_pretty(&check).expect("serialises");
    json_text.push('\n');
    for (o, kind) in &outputs {
        let bytes = if *kind == "output" { json_text.clone() } else { edit::edit_report(&check, &ir, lang) };
        write_output(o, bytes.as_bytes(), &inputs, p.flag("force"))?;
    }
    if p.flag("json") {
        print!("{json_text}");
    }
    if !p.flag("quiet") {
        for f in &check.findings {
            eprintln!("{}[{}]{}{}: {}", f.severity, f.code, f.change.as_ref().map(|c| format!(" {c}")).unwrap_or_default(), f.object.as_ref().map(|o| format!(" ({o})")).unwrap_or_default(), f.message);
        }
        eprintln!(
            "{}: {} - {} change(s): {} accepted, {} rejected{}",
            draft_path.display(),
            check.status.to_uppercase(),
            check.summary["changes"],
            check.summary["accepted"],
            check.summary["rejected"],
            if check.complete { String::new() } else { " (not everything was checked)".to_string() }
        );
    }
    match (check.status, check.complete) {
        ("rejected", _) => Err(CmdError::failed(format!("{}: the draft is rejected: {} change(s) outside what is authorised or breaking a rule", draft_path.display(), check.summary["rejected"]))),
        ("needs_review", _) => Err(CmdError::new(cmd::exit::PARTIAL, "the knowledge pack or the base design changed since the draft was made: it needs a review")),
        ("stopped", _) => Err(CmdError::new(cmd::exit::PARTIAL, "the generator stopped and reported what is missing (see the report)")),
        (_, false) => Err(CmdError::new(cmd::exit::PARTIAL, "no change is outside the whitelist, but parts could not be checked (base files not compared, or no design given)")),
        _ => Ok(()),
    }
}
