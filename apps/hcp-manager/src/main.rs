//! `hcp-manager`: sources, cache, text blocks, keyword search and evidence checks for hardware
//! constraint packs. Exit codes as in fastroute: 0 ok, 1 failed, 2 usage, 3 input, 4 output,
//! 5 partial (done, but something could not be done).

use clap::{Args, Parser, Subcommand};
use hcp_manager::error::{exit, Error, Result};
use hcp_manager::inventory::{self, Applies, Doc, Inventory};
use hcp_manager::workspace::Workspace;
use hcp_manager::{evidence, fetch, index, packs, search, sources, text, INTERFACE_VERSION};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "hcp-manager", version, about = "Documents, sources and knowledge packs for hardware constraint packs", long_about = None)]
struct Cli {
    /// The workspace directory (default: $HCP_MANAGER_HOME, else ./.hcp)
    #[arg(long, global = true, value_name = "DIR")]
    workspace: Option<PathBuf>,
    /// Print the result as JSON on standard output
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create the workspace directories
    Init,
    /// What this program offers (the contract the Skill negotiates against)
    Info,
    /// The inventory of documents and their cache
    Sources {
        #[command(subcommand)]
        cmd: SourcesCmd,
    },
    /// The derived text blocks (rebuildable)
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },
    /// Keyword search, filtered by device, version and source level before it is ranked
    Search(SearchArgs),
    /// Check that the evidence of a pack is in the cited documents
    Evidence {
        #[command(subcommand)]
        cmd: EvidenceCmd,
    },
    /// Knowledge packs in the workspace
    Packs {
        #[command(subcommand)]
        cmd: PacksCmd,
    },
}

#[derive(Subcommand)]
enum SourcesCmd {
    /// Add a document to the inventory
    Add(AddArgs),
    /// Add the documents of an inventory file (for example assets/example-sources.yaml)
    Import { file: PathBuf },
    /// List the documents; with --device, the ones usable for that device in order of source priority
    List(ListArgs),
    /// The documents for one device, best source first, with what is missing
    Resolve(ListArgs),
    /// Fetch the documents into the cache and pin their digests
    Sync(SyncArgs),
    /// Check the cache against the digests
    Check,
}

#[derive(Args)]
struct AddArgs {
    /// Document id, DOC-NAME
    #[arg(long)]
    id: String,
    #[arg(long)]
    title: String,
    /// official_manual, official_errata, official_guide, official_reference_design, official_devkit, open_board, community
    #[arg(long)]
    level: String,
    #[arg(long)]
    version: String,
    #[arg(long)]
    url: Option<String>,
    /// A local file (relative to the workspace, or absolute)
    #[arg(long, value_name = "PATH")]
    file: Option<String>,
    /// The exact device model(s) it is for: PART or PART:REVISION (repeat)
    #[arg(long = "applies-to", value_name = "PART[:REV]", required = true)]
    applies_to: Vec<String>,
    #[arg(long)]
    license_note: Option<String>,
    #[arg(long)]
    note: Option<String>,
}

#[derive(Args)]
struct ListArgs {
    #[arg(long)]
    device: Option<String>,
    #[arg(long)]
    revision: Option<String>,
    /// The document version that is wanted
    #[arg(long)]
    version: Option<String>,
}

#[derive(Args)]
struct SyncArgs {
    /// Only these documents (default: all)
    docs: Vec<String>,
    /// Adopt a document whose digest changed
    #[arg(long)]
    accept: bool,
    #[arg(long, default_value_t = 120)]
    timeout: u64,
}

#[derive(Subcommand)]
enum IndexCmd {
    /// Build the text blocks of the cached documents
    Build { docs: Vec<String> },
    /// Delete everything derived and build it again from the cache
    Rebuild,
    /// What is built, what is stale
    Status,
}

#[derive(Args)]
struct SearchArgs {
    /// The words to look for
    query: Vec<String>,
    /// The exact part number: documents of other models are not searched
    #[arg(long)]
    device: Option<String>,
    #[arg(long)]
    revision: Option<String>,
    /// Only documents of this version
    #[arg(long)]
    version: Option<String>,
    /// An exact level, or `official`
    #[arg(long = "source-level")]
    source_level: Option<String>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// keyword (default), or hybrid (falls back to keyword and says so)
    #[arg(long, default_value = "keyword")]
    mode: String,
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Verify sources, digests, block ids and quotations of a pack
    Verify { pack: String },
}

#[derive(Subcommand)]
enum PacksCmd {
    List,
    Show { pack: String },
    /// Write the pack with the inventory entries it cites into a directory
    Export {
        pack: String,
        #[arg(long, value_name = "DIR")]
        to: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

fn print_json(v: &impl serde::Serialize) {
    println!("{}", serde_json::to_string_pretty(v).expect("serialises"));
}

fn parse_applies(items: &[String]) -> Vec<Applies> {
    items
        .iter()
        .map(|s| match s.split_once(':') {
            Some((p, r)) => Applies { part_number: p.to_string(), revision: Some(r.to_string()) },
            None => Applies { part_number: s.clone(), revision: None },
        })
        .collect()
}

fn run(cli: Cli) -> Result<u8> {
    let ws = Workspace::resolve(cli.workspace.clone());
    let json = cli.json;
    match cli.command {
        Cmd::Init => {
            ws.init()?;
            if json {
                print_json(&json!({"format": "hcp-manager-init", "format_version": "1.0", "workspace": ws.root.display().to_string()}));
            } else {
                println!("workspace {}", ws.root.display());
            }
            Ok(0)
        }
        Cmd::Info => {
            let info = json!({
                "format": "hcp-manager-info",
                "format_version": "1.0",
                "name": "hcp-manager",
                "version": env!("CARGO_PKG_VERSION"),
                "interface": INTERFACE_VERSION,
                "commands": ["init", "info", "sources add|import|list|resolve|sync|check", "index build|rebuild|status", "search", "evidence verify", "packs list|show|export"],
                "capabilities": {
                    "keyword_search": true,
                    "vector_index": false,
                    "pdf_text": text::pdftotext_available(),
                    "network_fetch": fetch::curl_available(),
                },
                "exit_codes": {"ok": 0, "failed": 1, "usage": 2, "input": 3, "output": 4, "partial": 5},
            });
            if json {
                print_json(&info);
            } else {
                println!("hcp-manager {} (interface {INTERFACE_VERSION}); keyword search: yes, vector index: no, PDF text: {}, network fetch: {}", env!("CARGO_PKG_VERSION"), info["capabilities"]["pdf_text"], info["capabilities"]["network_fetch"]);
            }
            Ok(0)
        }
        Cmd::Sources { cmd } => run_sources(&ws, cmd, json),
        Cmd::Index { cmd } => run_index(&ws, cmd, json),
        Cmd::Search(a) => {
            let inv = Inventory::load(&ws)?;
            let q = search::Query { text: a.query.join(" "), device: a.device, revision: a.revision, version: a.version, source_level: a.source_level, limit: a.limit, mode: a.mode };
            let r = search::search(&ws, &inv, &q)?;
            if json {
                print_json(&r);
            } else {
                for w in &r.warnings {
                    eprintln!("warning: {w}");
                }
                if let Some(d) = &r.degraded {
                    eprintln!("note: {d}");
                }
                for e in &r.documents_excluded {
                    eprintln!("not searched: {} ({})", e.document_id, e.reason);
                }
                for h in &r.hits {
                    println!("{} [{} {}] p.{}{} score {}\n    {}", h.chunk_id, h.document_id, h.document_version, h.page, h.section.as_ref().map(|s| format!(", {s}")).unwrap_or_default(), h.score, h.snippet);
                }
                if r.hits.is_empty() {
                    println!("no hits ({} document(s) searched)", r.documents_considered.len());
                }
            }
            Ok(0)
        }
        Cmd::Evidence { cmd: EvidenceCmd::Verify { pack } } => {
            let inv = Inventory::load(&ws)?;
            let (path, doc) = packs::find(&ws, &pack)?;
            let r = evidence::verify(&ws, &inv, &doc, &path.display().to_string())?;
            if json {
                print_json(&r);
            } else {
                for p in &r.problems {
                    eprintln!("error[{}] {}: {}", p.code, p.path, p.message);
                }
                for u in &r.unverifiable {
                    eprintln!("not verified {}: {}", u.path, u.reason);
                }
                println!("{}: {} of {} evidence entries verified, {} problem(s), {} not verifiable", r.pack, r.verified, r.evidence_checked, r.problems.len(), r.unverifiable.len());
            }
            Ok(r.exit_code())
        }
        Cmd::Packs { cmd } => match cmd {
            PacksCmd::List => {
                let l = packs::list(&ws);
                if json {
                    print_json(&json!({"format": "hcp-manager-packs", "format_version": "1.0", "packs": l}));
                } else {
                    for p in &l {
                        println!("{} {} [{}] {}{}", p.id, p.version, p.review, p.devices.join(", "), if p.valid { "" } else { "  (NOT valid)" });
                    }
                    if l.is_empty() {
                        println!("no packs in {}", ws.packs_dir().display());
                    }
                }
                Ok(0)
            }
            PacksCmd::Show { pack } => {
                let (path, doc) = packs::find(&ws, &pack)?;
                print_json(&packs::show(&path, &doc));
                Ok(0)
            }
            PacksCmd::Export { pack, to, force } => {
                let (path, doc) = packs::find(&ws, &pack)?;
                let files = packs::export(&ws, &path, &doc, &to, force)?;
                if json {
                    print_json(&json!({"format": "hcp-manager-export-result", "format_version": "1.0", "directory": to.display().to_string(), "files": files}));
                } else {
                    println!("{}: {}", to.display(), files.join(", "));
                }
                Ok(0)
            }
        },
    }
}

fn run_sources(ws: &Workspace, cmd: SourcesCmd, json: bool) -> Result<u8> {
    match cmd {
        SourcesCmd::Add(a) => {
            sources::add(ws, Doc {
                document_id: a.id.clone(),
                title: a.title,
                level: a.level,
                version: a.version,
                applies_to: parse_applies(&a.applies_to),
                url: a.url,
                local_path: a.file,
                sha256: None,
                retrieved: None,
                status: None,
                license_note: a.license_note,
                note: a.note,
                last_error: None,
            })?;
            println!("added {}", a.id);
            Ok(0)
        }
        SourcesCmd::Import { file } => {
            let text = std::fs::read_to_string(&file).map_err(|e| Error::input(format!("{}: {e}", file.display())))?;
            let from = Inventory::parse(&text, &file.display().to_string())?;
            ws.init()?;
            let mut inv = if ws.inventory_path().exists() { Inventory::load(ws)? } else { Inventory::empty() };
            let mut added = 0;
            for d in from.documents {
                if inv.get(&d.document_id).is_none() {
                    inv.documents.push(d);
                    added += 1;
                }
            }
            inv.save(ws)?;
            println!("imported {added} document(s) from {}", file.display());
            Ok(0)
        }
        SourcesCmd::List(a) | SourcesCmd::Resolve(a) if a.device.is_some() => {
            let inv = Inventory::load(ws)?;
            let r = inventory::resolve(&inv, a.device.as_deref().unwrap_or(""), a.revision.as_deref(), a.version.as_deref());
            if json {
                print_json(&r);
            } else {
                for d in &r.documents {
                    println!("{} {} {} [{}]{}", if d.usable { "use " } else { "skip" }, d.document_id, d.version, d.level, if d.notes.is_empty() { String::new() } else { format!(" - {}", d.notes.join("; ")) });
                }
                for s in &r.similar_models_excluded {
                    println!("other model, not used: {s}");
                }
                for m in &r.missing {
                    println!("missing: no usable {m} for {}", r.device);
                }
                for w in &r.warnings {
                    eprintln!("warning: {w}");
                }
            }
            Ok(if r.missing.contains(&"official_manual".to_string()) { exit::PARTIAL } else { 0 })
        }
        SourcesCmd::List(_) | SourcesCmd::Resolve(_) => {
            let inv = Inventory::load(ws)?;
            if json {
                print_json(&inv);
            } else {
                for d in &inv.documents {
                    println!("{} {} [{}] for {} {}", d.document_id, d.version, d.level, d.applies_to.iter().map(|a| a.part_number.clone()).collect::<Vec<_>>().join(", "), d.status.clone().unwrap_or_else(|| if d.sha256.is_some() { "active".into() } else { "not cached".into() }));
                }
            }
            Ok(0)
        }
        SourcesCmd::Sync(a) => {
            let r = sources::sync(ws, &a.docs, a.accept, a.timeout)?;
            if json {
                print_json(&r);
            } else {
                for x in &r.results {
                    println!("{} {}: {}", x.document_id, format!("{:?}", x.status).to_lowercase(), x.message);
                }
                for w in &r.warnings {
                    eprintln!("warning: {w}");
                }
            }
            Ok(if r.results.iter().any(|x| x.status == "unreachable" || x.status == "changed") { exit::PARTIAL } else { 0 })
        }
        SourcesCmd::Check => {
            let p = sources::check(ws)?;
            if json {
                print_json(&json!({"format": "hcp-manager-cache-check", "format_version": "1.0", "problems": p}));
            } else if p.is_empty() {
                println!("the cache matches the inventory");
            } else {
                for x in &p {
                    eprintln!("{}: {}", x.document_id, x.problem);
                }
            }
            Ok(if p.is_empty() { 0 } else { 1 })
        }
    }
}

fn run_index(ws: &Workspace, cmd: IndexCmd, json: bool) -> Result<u8> {
    let inv = Inventory::load(ws)?;
    match cmd {
        IndexCmd::Build { docs } => report_build(index::build(ws, &inv, &docs)?, json),
        IndexCmd::Rebuild => report_build(index::rebuild(ws, &inv)?, json),
        IndexCmd::Status => {
            let s = index::status(ws, &inv);
            if json {
                print_json(&s);
            } else {
                println!("keyword index: {}; vector index: {}", s.keyword_index, s.vector_index);
                for d in &s.documents {
                    println!("{} {} ({} blocks)", d.document_id, d.state, d.chunks);
                }
            }
            Ok(0)
        }
    }
}

fn report_build(results: Vec<(String, std::result::Result<usize, String>)>, json: bool) -> Result<u8> {
    let failed = results.iter().any(|(_, r)| r.is_err());
    if json {
        let items: Vec<Value> = results.iter().map(|(d, r)| match r {
            Ok(n) => json!({"document_id": d, "chunks": n}),
            Err(e) => json!({"document_id": d, "error": e}),
        }).collect();
        print_json(&json!({"format": "hcp-manager-index-build", "format_version": "1.0", "documents": items}));
    } else {
        for (d, r) in &results {
            match r {
                Ok(n) => println!("{d}: {n} blocks"),
                Err(e) => eprintln!("{d}: no blocks: {e}"),
            }
        }
    }
    Ok(if failed { exit::PARTIAL } else { 0 })
}

fn main() -> ExitCode {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { exit::USAGE } else { 0 };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("hcp-manager: {}", e.message);
            ExitCode::from(e.code)
        }
    }
}
