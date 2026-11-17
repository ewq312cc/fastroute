//! `hcp-manager`: sources, cache, text blocks, keyword search and evidence checks for hardware
//! constraint packs. Exit codes as in fastroute: 0 ok, 1 failed, 2 usage, 3 input, 4 output,
//! 5 partial (done, but something could not be done).

use clap::{Args, Parser, Subcommand};
use hcp_manager::error::{exit, Error, Result};
use hcp_manager::inventory::{self, Applies, Doc, Inventory};
use hcp_manager::workspace::Workspace;
use hcp_manager::store::Store;
use hcp_manager::{embed, evidence, fetch, index, packs, release, search, sources, text, INTERFACE_VERSION};
use serde_json::json;
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
    /// The embedding profile for this command (default: `active_profile` of embedding.yaml)
    #[arg(long, global = true, value_name = "NAME")]
    profile: Option<String>,
    /// The vector store backend for this command: sqlite (default) or jsonl (default: `store.backend` of embedding.yaml)
    #[arg(long, global = true, value_name = "NAME")]
    backend: Option<String>,
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
    /// The derived index: text blocks and vectors in the embedded store (rebuildable)
    Index {
        #[command(subcommand)]
        cmd: IndexCmd,
    },
    /// Keyword, vector or hybrid search, filtered by device, version and source level before it is ranked
    Search(SearchArgs),
    /// One text block, with everything needed to go back to the original page
    Chunk {
        #[command(subcommand)]
        cmd: ChunkCmd,
    },
    /// The embedding provider of the workspace (default: local, nothing leaves the machine)
    Embedding {
        #[command(subcommand)]
        cmd: EmbeddingCmd,
    },
    /// Versioned packs that ship with the software: seal, catalogue and build a release (for pack authors and release builders)
    Release {
        #[command(subcommand)]
        cmd: ReleaseCmd,
    },
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
    /// Allow the text of this document to be sent to a remote embedding service (default: no)
    #[arg(long)]
    embed_remote: bool,
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
    /// Build what is missing or stale: the text blocks of the cached documents and their vectors
    Build {
        /// Only these documents (default: all)
        docs: Vec<String>,
        /// Make everything of the selected documents again, current or not
        #[arg(long)]
        force: bool,
        /// Build the text blocks only
        #[arg(long)]
        no_vector: bool,
        /// Allow a remote embedding provider to receive text (the configuration must allow it too)
        #[arg(long)]
        authorize_remote: bool,
    },
    /// Delete everything derived and build it again from the cache
    Rebuild {
        #[arg(long)]
        no_vector: bool,
        #[arg(long)]
        authorize_remote: bool,
    },
    /// What is built, what is stale and why; which backend and profile are in force; the generations of vectors in the store
    Status,
    /// Remove the vectors of every embedding configuration except the one in force (older generations are otherwise kept, never deleted by a build)
    Prune,
}

#[derive(Subcommand)]
enum ChunkCmd {
    /// Print a block (a chunk id from a search hit or from evidence) with its document, version, page, section and digests
    Show { id: String },
}

#[derive(Subcommand)]
enum EmbeddingCmd {
    /// The configuration in force, what the vectors are made with, and whether text would leave this machine
    Show,
    /// Embed two sample sentences and report the dimension and the similarity (needs --authorize-remote for a remote provider)
    Test {
        #[arg(long)]
        authorize_remote: bool,
    },
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
    /// keyword (default), vector (an error if there are no usable vectors), or hybrid (keyword and vector; keyword alone, and says
    /// so, if there are no usable vectors)
    #[arg(long, default_value = "keyword")]
    mode: String,
    /// Allow a remote embedding provider to receive the query (the configuration must allow it too)
    #[arg(long)]
    authorize_remote: bool,
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Verify sources, digests, block ids and quotations of a pack
    Verify { pack: String },
}

#[derive(Subcommand)]
enum ReleaseCmd {
    /// Write the computed part of a pack directory's manifest: digests of the files, the canonical digest of hcp.yaml, the locked sources
    Seal { dir: PathBuf },
    /// Print the canonical digest of an HCP document (what a pack that inherits from it pins in pack.upstream)
    Digest { file: PathBuf },
    /// Write (or, with --check, verify) the catalogue index.yaml of the packs in a tree: the lock of what a release contains
    Index {
        #[arg(long, default_value = "hardware-packs")]
        packs: PathBuf,
        /// Do not write: fail if the committed index.yaml is not the current catalogue
        #[arg(long)]
        check: bool,
    },
    /// Build the release directory (release.yaml, SHA256SUMS, one deterministic archive per pack) from a tree of packs
    Build {
        #[arg(long, default_value = "hardware-packs")]
        packs: PathBuf,
        #[arg(long, default_value = "dist/hcp-packs")]
        out: PathBuf,
        /// Replace an existing release directory
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum BuiltinCmd {
    /// The packs of a release, and whether each is installed in the workspace (installed / modified / not installed)
    List,
    /// One pack of the release: identity, kind, version, dependencies, compatibility, licence and digests
    Show { pack: String },
    /// Verify the release offline: checksums, every archive, every pack, the dependencies and the merged documents
    Verify,
    /// Install a pack and the exact packs it depends on into the workspace, from the archives
    Install {
        pack: String,
        /// Replace a locally changed or differently installed copy (it is moved to backup/ first)
        #[arg(long)]
        force: bool,
    },
}

#[derive(Subcommand)]
enum PacksCmd {
    /// The packs shipped with this software (a release directory; offline)
    Builtin {
        /// The release directory (default: $HCP_PACKS_DIR, <program>/../share/hcp-packs, ./dist/hcp-packs)
        #[arg(long, global = true, value_name = "DIR")]
        release: Option<PathBuf>,
        #[command(subcommand)]
        cmd: BuiltinCmd,
    },
    /// Embed a development board pack in a carrier: the carrier pack, its IR, the differences and a report; the board stays fixed
    Embed {
        /// The board pack: path, id (the highest version) or id@version of the workspace
        #[arg(long)]
        board: String,
        /// The embedding request (JSON or YAML), see schemas/board-embedding-request-v1.schema.json
        #[arg(long, value_name = "FILE")]
        request: PathBuf,
        /// The directory for embedding.json, carrier-pack.yaml, ir.json, diff-report.json and the report
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
        #[arg(long)]
        force: bool,
        /// The language of the report: zh (default) or en
        #[arg(long, default_value = "zh")]
        lang: String,
    },
    /// A pack that inherits (chip -> module -> board) merged with the exact upstream packs of the workspace into one flat pack
    Resolve {
        pack: String,
        /// Write the flat pack here (default: standard output)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// The packs of the workspace (every version of every pack)
    List,
    /// A pack by path, id (the highest version) or id@version
    Show { pack: String },
    /// Make a skeleton pack for a device: metadata, the device and the documents of the inventory that are for it (no pins, no rules)
    Create {
        /// vendor/device
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "0.1.0")]
        pack_version: String,
        /// The exact part number
        #[arg(long)]
        device: String,
        #[arg(long)]
        revision: Option<String>,
        #[arg(long)]
        vendor: String,
        #[arg(long)]
        title: Option<String>,
        /// zh-CN or en
        #[arg(long, default_value = "en")]
        language: String,
    },
    /// Import a pack file, or a directory written by `packs export`, with the documents it cites
    Import {
        from: PathBuf,
        #[arg(long)]
        force: bool,
    },
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
    let mut ws = Workspace::resolve(cli.workspace.clone());
    ws.profile = cli.profile.clone();
    ws.backend = cli.backend.clone();
    if let Some(b) = &ws.backend {
        hcp_manager::store::Kind::parse(b)?;
    }
    let ws = ws;
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
                "format_version": "1.1",
                "name": "hcp-manager",
                "version": env!("CARGO_PKG_VERSION"),
                "interface": INTERFACE_VERSION,
                "commands": [
                    "init", "info", "sources add|import|list|resolve|sync|check", "index build|rebuild|status|prune", "search", "chunk show",
                    "embedding show|test", "evidence verify", "packs list|show|create|import|export|resolve|embed", "packs builtin list|show|verify|install", "release seal|index|build"
                ],
                "capabilities": {
                    "keyword_search": true,
                    "vector_index": true,
                    "hybrid_search": true,
                    "embedded_store": "sqlite",
                    "vector_backends": ["sqlite", "jsonl"],
                    "embedding_profiles": true,
                    "builtin_packs": true,
                    "pack_inheritance": true,
                    "release_format": release::RELEASE_FORMAT,
                    "pack_manifest_version": release::MANIFEST_VERSION,
                    "embedding_providers": ["hashing", "command", "http", "openai"],
                    "default_embedding_provider": "hashing",
                    "semantic_model_bundled": false,
                    "remote_embedding_by_default": false,
                    "pdf_text": text::pdftotext_available(),
                    "network_fetch": fetch::curl_available(),
                },
                "search_modes": ["keyword", "vector", "hybrid"],
                "exit_codes": {"ok": 0, "failed": 1, "usage": 2, "input": 3, "output": 4, "partial": 5},
            });
            if json {
                print_json(&info);
            } else {
                println!(
                    "hcp-manager {} (interface {INTERFACE_VERSION}); keyword search: yes, vector index: yes (local hashing by default; no model is bundled), PDF text: {}, network fetch: {}",
                    env!("CARGO_PKG_VERSION"),
                    info["capabilities"]["pdf_text"],
                    info["capabilities"]["network_fetch"]
                );
            }
            Ok(0)
        }
        Cmd::Sources { cmd } => run_sources(&ws, cmd, json),
        Cmd::Index { cmd } => run_index(&ws, cmd, json),
        Cmd::Search(a) => {
            let inv = Inventory::load(&ws)?;
            let q = search::Query { text: a.query.join(" "), device: a.device, revision: a.revision, version: a.version, source_level: a.source_level, limit: a.limit, mode: a.mode, authorize_remote: a.authorize_remote };
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
                    println!("{} [{} {}] p.{}{} score {} ({})\n    {}", h.chunk_id, h.document_id, h.document_version, h.page, h.section.as_ref().map(|s| format!(", {s}")).unwrap_or_default(), h.score, h.matched_by.join("+"), h.snippet);
                }
                if r.hits.is_empty() {
                    println!("no hits ({} document(s) searched)", r.documents_considered.len());
                }
            }
            Ok(0)
        }
        Cmd::Chunk { cmd: ChunkCmd::Show { id } } => {
            let store = Store::open_read(&ws).ok_or_else(|| Error::input("there is no index store to read: run `hcp-manager index build`"))?;
            let rec = store.chunk(&id)?.ok_or_else(|| Error::input(format!("no block {id} in the index (a block id contains the digest of the document it was cut from: after a document changed, the old ids are gone)")))?;
            let inv = Inventory::load(&ws).ok();
            let doc = inv.as_ref().and_then(|i| i.get(&rec.document_id));
            let meta = index::read_meta(&ws, &rec.document_id);
            let current = match (doc, &meta) {
                (Some(d), Some(m)) => index::keyword_state(store.doc_row(&rec.document_id)?.as_ref(), Some(m), d).0 == "indexed",
                _ => false,
            };
            let out = json!({
                "format": "hcp-manager-chunk",
                "format_version": "1.0",
                "chunk": rec,
                "source": meta.as_ref().map(|m| json!({"document_id": m.document_id, "sha256": m.sha256, "cached_file": index::cached_file(&ws, m).display().to_string(), "retrieved": m.retrieved, "origin": m.origin})),
                "current": current,
                "evidence": {"source_document": rec.document_id, "locator": match &rec.section { Some(s) => json!({"page": rec.page, "section": s}), None => json!({"page": rec.page}) }, "quote": "(read the text above and copy the exact words you rely on)"},
            });
            if json {
                print_json(&out);
            } else {
                println!("{} [{} {} {}] page {}{}{}", rec.chunk_id, rec.document_id, rec.document_version, rec.level, rec.page, rec.section.as_ref().map(|s| format!(", {s}")).unwrap_or_default(), if current { "" } else { "  (the index of this document is not current)" });
                println!("for: {}\ndigest of the text: {}\n\n{}", rec.applies_to.join(", "), rec.text_sha256, rec.text);
            }
            Ok(0)
        }
        Cmd::Embedding { cmd } => run_embedding(&ws, cmd, json),
        Cmd::Evidence { cmd: EvidenceCmd::Verify { pack } } => {
            let inv = Inventory::load(&ws)?;
            let (path, doc) = packs::find(&ws, &pack)?;
            let doc = packs::resolved_doc(&ws, &doc).map_err(|e| Error::input(format!("{}: the pack does not resolve:\n  {}", path.display(), e.join("\n  "))))?;
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
        Cmd::Release { cmd } => run_release(cmd, json),
        Cmd::Packs { cmd } => match cmd {
            PacksCmd::Builtin { release, cmd } => run_builtin(&ws, release, cmd, json),
            PacksCmd::Embed { board, request, out, force, lang } => {
                if lang != "zh" && lang != "en" {
                    return Err(Error::usage("--lang is zh or en"));
                }
                let r = packs::embed(&ws, &board, &request, &out, force, lang == "zh")?;
                if json {
                    print_json(&r);
                } else {
                    println!("{}: {} ({} header pins usable, {} editable areas) -> {}", r.board, r.status, r.pin_pool, r.editable_regions, r.out);
                    println!("{}", r.files.join(", "));
                }
                Ok(if r.status == "ok" { 0 } else { exit::PARTIAL })
            }
            PacksCmd::Resolve { pack, output } => {
                let (path, doc) = packs::find(&ws, &pack)?;
                let flat = packs::resolved_doc(&ws, &doc).map_err(|e| Error::input(format!("{}: not resolved:\n  {}", path.display(), e.join("\n  "))))?;
                let text = format!("# Resolved by `hcp-manager packs resolve`: the pack and what it inherits, merged. Generated: edit the pack, not this file.\n{}", fr_constraint::hcp::to_yaml(&flat).map_err(Error::failed)?);
                match output {
                    Some(o) => {
                        if o.exists() && std::fs::canonicalize(&o).ok() == std::fs::canonicalize(&path).ok() {
                            return Err(Error::output(format!("{} is the pack itself: it is not overwritten", o.display())));
                        }
                        hcp_manager::workspace::write_atomic(&o, text.as_bytes())?;
                        if !json {
                            println!("{}", o.display());
                        }
                    }
                    None => print!("{text}"),
                }
                Ok(0)
            }
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
            PacksCmd::Create { id, pack_version, device, revision, vendor, title, language } => {
                ws.init()?;
                let inv = if ws.inventory_path().exists() { Inventory::load(&ws)? } else { Inventory::empty() };
                let (path, r) = packs::create(&ws, &inv, &packs::CreateSpec { id, version: pack_version, title, vendor, part_number: device, revision, language })?;
                if json {
                    print_json(&r);
                } else {
                    println!("{}: a skeleton with {} source(s) from the inventory", path.display(), r.sources.len());
                    for n in &r.not_included {
                        println!("not included: {n}");
                    }
                    println!("not valid yet (as expected): {}", r.errors.first().cloned().unwrap_or_default());
                }
                Ok(0)
            }
            PacksCmd::Import { from, force } => {
                let r = packs::import(&ws, &from, force)?;
                if json {
                    print_json(&r);
                } else {
                    println!("{} {}: {}{}", r.pack_id, r.version, r.path, if r.already_imported { " (it was already there)" } else { "" });
                    if !r.inventory_added.is_empty() {
                        println!("added to the inventory: {}", r.inventory_added.join(", "));
                    }
                    for m in &r.missing_from_inventory {
                        eprintln!("warning: the pack cites {m}, which is not in the inventory");
                    }
                }
                Ok(if r.missing_from_inventory.is_empty() { 0 } else { exit::PARTIAL })
            }
            PacksCmd::Show { pack } => {
                let (path, doc) = packs::find(&ws, &pack)?;
                print_json(&packs::show(&ws, &path, &doc));
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

fn run_release(cmd: ReleaseCmd, json: bool) -> Result<u8> {
    match cmd {
        ReleaseCmd::Seal { dir } => {
            let m = release::seal(&dir)?;
            if json {
                print_json(&m);
            } else {
                println!("{}: sealed {} {} ({} files)", dir.join("pack.yaml").display(), m["id"].as_str().unwrap_or(""), m["version"].as_str().unwrap_or(""), 2 + m["files"]["reports"].as_array().map(Vec::len).unwrap_or(0) + m["files"]["documents"].as_array().map(Vec::len).unwrap_or(0));
            }
            Ok(0)
        }
        ReleaseCmd::Digest { file } => {
            let doc = packs::load(&file)?;
            let d = fr_constraint::resolve::canonical_sha256(&doc);
            if json {
                print_json(&json!({"format": "hcp-manager-digest", "format_version": "1.0", "file": file.display().to_string(), "canonical_sha256": d}));
            } else {
                println!("{d}");
            }
            Ok(0)
        }
        ReleaseCmd::Index { packs, check } => {
            let (all, problems) = release::load_tree(&packs)?;
            if !problems.is_empty() {
                for p in &problems {
                    eprintln!("error[{}] {}: {}", p.code, p.pack, p.message);
                }
                return Err(Error::input(format!("{}: {} problem(s): the catalogue is not written", packs.display(), problems.len())));
            }
            let text = release::index_text(&all)?;
            let target = packs.join("index.yaml");
            if check {
                let have = std::fs::read_to_string(&target).map_err(|e| Error::input(format!("{}: {e}", target.display())))?;
                if have != text {
                    return Err(Error::input(format!("{} is not the current catalogue of the packs in the tree: run `hcp-manager release index` and review the change", target.display())));
                }
                if !json {
                    println!("{}: current ({} packs)", target.display(), all.len());
                }
            } else {
                hcp_manager::workspace::write_atomic(&target, text.as_bytes())?;
                if !json {
                    println!("{}: {} packs", target.display(), all.len());
                }
            }
            if json {
                print_json(&json!({"format": "hcp-manager-release-index", "format_version": "1.0", "index": target.display().to_string(), "packs": all.len(), "checked": check}));
            }
            Ok(0)
        }
        ReleaseCmd::Build { packs, out, force } => {
            let r = release::build(&packs, &out, force)?;
            if json {
                print_json(&r);
            } else {
                println!("{}: {} packs", r.out, r.packs);
            }
            Ok(0)
        }
    }
}

fn run_builtin(ws: &Workspace, release_dir: Option<PathBuf>, cmd: BuiltinCmd, json: bool) -> Result<u8> {
    let dir = release::Release::locate(release_dir.as_deref())?;
    let rel = release::Release::open(&dir)?;
    match cmd {
        BuiltinCmd::List => {
            let rows: Vec<serde_json::Value> = rel.entries().iter().map(|e| json!({"id": e["id"], "version": e["version"], "kind": e["kind"], "content": e["content"], "requires": e["requires"], "installed": release::state_of(ws, e)})).collect();
            if json {
                print_json(&json!({"format": "hcp-manager-builtin-list", "format_version": "1.0", "release": dir.display().to_string(), "software_version": rel.doc["software"]["version"], "packs": rows}));
            } else {
                for r in &rows {
                    println!("{} {} [{}] {} ({})", r["id"].as_str().unwrap_or(""), r["version"].as_str().unwrap_or(""), r["kind"].as_str().unwrap_or(""), r["content"].as_str().unwrap_or(""), r["installed"].as_str().unwrap_or(""));
                }
            }
            Ok(0)
        }
        BuiltinCmd::Show { pack } => {
            let e = rel.find(&pack)?;
            let p = rel.pack(e).map_err(|ps| Error::input(ps.iter().map(|p| format!("[{}] {}", p.code, p.message)).collect::<Vec<_>>().join("\n")))?;
            print_json(&json!({"format": "hcp-manager-builtin-show", "format_version": "1.0", "entry": e, "manifest": p.manifest, "installed": release::state_of(ws, e)}));
            Ok(0)
        }
        BuiltinCmd::Verify => {
            let (packs, problems) = rel.verify()?;
            if json {
                print_json(&json!({"format": "hcp-manager-builtin-verify", "format_version": "1.0", "release": dir.display().to_string(), "packs": packs.len(), "ok": problems.is_empty(), "problems": problems}));
            } else {
                for p in &problems {
                    eprintln!("error[{}] {}: {}", p.code, p.pack, p.message);
                }
                println!("{}: {} packs, {} problem(s)", dir.display(), packs.len(), problems.len());
            }
            Ok(if problems.is_empty() { 0 } else { exit::INPUT })
        }
        BuiltinCmd::Install { pack, force } => {
            let r = release::install(ws, &rel, &pack, force)?;
            if json {
                print_json(&r);
            } else {
                for p in &r.packs {
                    println!("{} {} [{}]: {} ({})", p.id, p.version, p.kind, p.path, p.action);
                }
                if !r.inventory_added.is_empty() {
                    println!("added to the inventory: {}", r.inventory_added.join(", "));
                }
            }
            Ok(0)
        }
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
                embed_remote: if a.embed_remote { Some(true) } else { None },
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
        IndexCmd::Build { docs, force, no_vector, authorize_remote } => report_build(index::build(ws, &inv, &docs, &index::BuildOptions { force, no_vector, authorize_remote })?, json),
        IndexCmd::Rebuild { no_vector, authorize_remote } => report_build(index::rebuild(ws, &inv, &index::BuildOptions { force: true, no_vector, authorize_remote })?, json),
        IndexCmd::Prune => {
            let r = index::prune(ws)?;
            if json {
                print_json(&r);
            } else {
                println!("{}: kept {}, removed {} older generation(s) of vectors", r.backend, r.kept, r.removed_generations);
            }
            Ok(0)
        }
        IndexCmd::Status => {
            let s = index::status(ws, &inv);
            if json {
                print_json(&s);
            } else {
                println!("keyword index: {}; vector index: {}; store: {} ({}, {})", s.keyword_index, s.vector_index, s.store.state, s.store.backend, s.store.path);
                if let Some(r) = &s.store.reason {
                    println!("store: {r}");
                }
                if let Some(e) = &s.embedder.configured {
                    println!("embedding: {} ({}{}), profile {}", e.key, e.provider, if e.remote { ", REMOTE" } else { ", local" }, s.embedder.profile.as_deref().unwrap_or("?"));
                }
                for g in s.embedder.generations.iter().filter(|g| !g.in_force) {
                    println!("older generation kept: {} ({} vectors); `index prune` removes it", g.embedder_key, g.vectors);
                }
                if let Some(p) = &s.embedder.problem {
                    println!("embedding problem: {p}");
                }
                for d in &s.documents {
                    println!(
                        "{} {} ({} blocks){}; vectors {}{}",
                        d.document_id,
                        d.state,
                        d.chunks,
                        d.stale_reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default(),
                        d.vector.state,
                        d.vector.reason.as_ref().map(|r| format!(": {r}")).unwrap_or_default()
                    );
                }
            }
            Ok(0)
        }
    }
}

fn report_build(r: index::BuildReport, json: bool) -> Result<u8> {
    if json {
        print_json(&r);
    } else {
        if let Some(n) = &r.store_note {
            eprintln!("note: {n}");
        }
        if let Some(p) = &r.embedder_problem {
            eprintln!("no vectors: {p}");
        }
        for d in &r.documents {
            match &d.error {
                Some(e) => eprintln!("{}: no blocks: {e}", d.document_id),
                None => println!(
                    "{}: {} blocks ({}); vectors {}{}",
                    d.document_id,
                    d.chunks,
                    d.keyword,
                    d.vectors.state,
                    d.vectors.reason.as_ref().map(|x| format!(": {x}")).unwrap_or_default()
                ),
            }
        }
    }
    Ok(if r.complete() { 0 } else { exit::PARTIAL })
}

fn run_embedding(ws: &Workspace, cmd: EmbeddingCmd, json: bool) -> Result<u8> {
    match cmd {
        EmbeddingCmd::Show => {
            let settings = embed::load_settings(ws)?;
            let cfg = settings.config.clone();
            let info = embed::describe(&cfg)?;
            // what the configuration says, without anything that could be a secret: the key is only ever named (an environment variable,
            // or the program that prints it), never shown
            let mut shown = serde_json::to_value(&cfg).expect("serialises");
            if let Some(c) = cfg.api_key_command.as_ref().and_then(|c| c.first()) {
                shown["api_key_command"] = json!([c, "..."]);
            }
            let key_source = match (&cfg.api_key_env, &cfg.api_key_command) {
                (Some(e), _) => format!("environment variable {e}"),
                (_, Some(c)) => format!("the output of {}", c.first().cloned().unwrap_or_default()),
                _ => "none (no Authorization header is sent)".to_string(),
            };
            let kind = hcp_manager::store::Store::kind(ws)?;
            let out = json!({
                "format": "hcp-manager-embedding",
                "format_version": "1.1",
                "config_file": ws.embedding_config_path().display().to_string(),
                "config_file_present": ws.embedding_config_path().exists(),
                "profile": settings.profile,
                "profiles": settings.profiles,
                "store_backend": kind.name(),
                "store_location": kind.location(ws).display().to_string(),
                "config": shown,
                "embedder": info,
                "api_key_source": key_source,
                "text_leaves_this_machine": info.remote,
                "remote_requires": if info.remote { json!(["allow_remote: true in embedding.yaml", "--authorize-remote on the command", "embed_remote: true in the inventory entry of each document that may be sent"]) } else { json!([]) },
                "note": if info.provider == "hashing" { "the hashing provider is lexical (words, word pairs and character trigrams); a semantic model is plugged in with provider `openai`, `command` or `http`" } else { "" },
            });
            if json {
                print_json(&out);
            } else {
                println!("profile {} ({} defined): provider {} model {} version {} dim {}; text leaves this machine: {}", settings.profile, settings.profiles.len(), info.provider, info.model, info.version, info.dim, if info.remote { "YES (remote)" } else { "no" });
                if let Some(e) = &info.endpoint {
                    println!("endpoint: {e}; API key: {key_source}");
                }
                println!("store backend: {} ({})", kind.name(), kind.location(ws).display());
                println!("config file: {} ({})", ws.embedding_config_path().display(), if ws.embedding_config_path().exists() { "present" } else { "absent: the local default is used" });
            }
            Ok(0)
        }
        EmbeddingCmd::Test { authorize_remote } => {
            let cfg = embed::load_config(ws)?;
            let e = embed::make(&cfg, authorize_remote, &ws.root)?;
            let v = e.embed(&["IO0 is sampled at reset".to_string(), "the strapping pin IO0 is read when the chip resets".to_string(), "keep the antenna area free of copper".to_string()])?;
            let out = json!({
                "format": "hcp-manager-embedding-test",
                "format_version": "1.0",
                "embedder": e.info(),
                "dim": v[0].len(),
                "similarity": {"related_sentences": embed::cosine(&v[0], &v[1]), "unrelated_sentences": embed::cosine(&v[0], &v[2])},
            });
            if json {
                print_json(&out);
            } else {
                println!("{}: dim {}; similarity related {:.3}, unrelated {:.3}", e.info().key, v[0].len(), embed::cosine(&v[0], &v[1]), embed::cosine(&v[0], &v[2]));
            }
            Ok(0)
        }
    }
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
