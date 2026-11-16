//! `fastroute constraints`: Hardware Constraint Packs (HCP) - check, compile, validate.
//!
//! ```text
//! constraints schema        the JSON Schema of a pack
//! constraints check PACK    the schema and the semantic checks of a pack (exit 3 if it is not valid)
//! ```
//!
//! Everything is deterministic and offline: no AI model, no network, no vector index. The format
//! is described in `skills/hardware-constraint-design/references/hcp-format.md`.

use std::path::{Path, PathBuf};

use fr_constraint::diag::{Diagnostic, Severity};
use fr_constraint::hcp;
use serde_json::{json, Value};

use crate::cmd::{check_output, opt, write_output, CmdError, Opt, Parsed, Spec};
use crate::load::sha256_hex;

const NO_OPTS: &[Opt] = &[];
const CHECK_OPTS: &[Opt] = &[
    opt("pack", None, Some("FILE"), "the pack, YAML or JSON (also the first positional argument)"),
    opt("json", None, None, "print the report as JSON on standard output"),
    opt("output", Some('o'), Some("FILE"), "write the JSON report here"),
    opt("force", None, None, "replace an existing output file"),
    opt("quiet", Some('q'), None, "no diagnostics on standard error"),
];

const fn spec(name: &'static str, about: &'static str, usage: &'static str, opts: &'static [Opt], footer: &'static str) -> Spec {
    Spec { name, about, usage, opts, passthrough: &[], footer }
}

const SCHEMA_SPEC: Spec = spec("constraints schema", "print the JSON Schema of a hardware constraint pack", "constraints schema", NO_OPTS, "The same file is skills/hardware-constraint-design/schemas/hcp-v1.schema.json; `constraints check` enforces it.");
const CHECK_SPEC: Spec = spec(
    "constraints check",
    "check a pack: structure, references, evidence, units, whitelist of what is editable",
    "constraints check PACK [--json] [--output report.json]",
    CHECK_OPTS,
    "Exit 0 if the pack is valid (warnings are listed), 3 if it is not. Needs no network, AI model or vector index.\nDiagnostic codes: skills/hardware-constraint-design/references/hcp-format.md.",
);

const SUBCOMMANDS: &str = "\
commands:
  schema              print the JSON Schema of a pack
  check               check a pack (exit 3 if it is not valid)

Run `fastroute constraints <command> --help` for the options. Format: skills/hardware-constraint-design/references/hcp-format.md.
";

pub fn help_text() -> String {
    format!("fastroute constraints - hardware constraint packs (HCP): deterministic, offline checks\n\nusage: fastroute constraints <command> [options]\n\n{SUBCOMMANDS}")
}

pub fn help_for(sub: Option<&str>) -> String {
    match sub {
        Some("schema") => SCHEMA_SPEC.help_text(),
        Some("check") => CHECK_SPEC.help_text(),
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
        other => Err(CmdError::usage(format!("unknown command 'constraints {other}' (schema, check)"))),
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
    let Some(_) = parse(&SCHEMA_SPEC, args)? else { return Ok(()) };
    print!("{}", hcp::SCHEMA_TEXT);
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
