//! The derived index: the text blocks of the cached documents, their vectors and a record of what each was built from (the digest of the
//! source, the chunking strategy, the entry of the inventory, the embedding configuration). It is derived data: delete it and
//! `index build` makes it again from the cache. There is no server, and nothing in a pack, an IR or a verdict reads it.
//!
//! The persistence is behind the [`Backend`] trait. Two backends ship, each with its own location and never sharing a file:
//!
//! * `sqlite` (default): one file, `derived/index.db`.
//! * `jsonl`: a directory of JSON Lines files, `derived/jsonl/` (a manifest with the format version and the SHA-256 and line count of every
//!   file, so damage is detected). It exists to show that the stored format is replaceable and to be read with any tool; it holds everything
//!   in memory while open and rewrites files on change, so it is for small workspaces and debugging, not a fast store.
//!
//! Vectors are kept per embedding configuration (its key): another model, dimension or endpoint makes a **new generation**, the old one
//! stays (read-only in effect: searches use the configuration in force) until `index prune` removes it. The nearest-neighbour search is the
//! flat scan over the blocks that survived the filters (device, version, source level); an ANN backend implements the same trait.

use crate::error::{Error, Result};
use crate::text::Chunk;
use crate::workspace::Workspace;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// The layout of the tables. A store of another version is not migrated, it is rebuilt.
pub const STORE_VERSION: i64 = 1;

/// How the blocks are cut: paragraph boundaries, 400 to 1500 characters (see `text::chunk_pages`). Part of the record of every
/// document: a block set made with another strategy is stale.
pub const CHUNKER: &str = "paragraph-400-1500-v1";

/// A text block with everything that is kept about it: the document, its version and source level, the devices it is for, the page
/// and section, and the digest of the text. This is the record that `chunk show` prints and that a hit points to.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub chunk_id: String,
    pub document_id: String,
    pub document_version: String,
    pub level: String,
    /// `PART` or `PART REV`
    pub applies_to: Vec<String>,
    pub page: u32,
    pub ordinal: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    pub text: String,
    /// SHA-256 of the text of the block
    pub text_sha256: String,
}

impl ChunkRecord {
    pub fn as_chunk(&self) -> Chunk {
        Chunk { chunk_id: self.chunk_id.clone(), document_id: self.document_id.clone(), page: self.page, ordinal: self.ordinal, section: self.section.clone(), text: self.text.clone() }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DocRow {
    pub document_id: String,
    pub sha256: String,
    pub chunker: String,
    pub inv_sig: String,
    pub chunks: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbedderRow {
    pub embedder_key: String,
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dim: usize,
    pub config_sha256: String,
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VecRow {
    pub document_id: String,
    pub embedder_key: String,
    pub sha256: String,
    pub chunker: String,
    pub vectors: usize,
}

/// What the file on disk is.
#[derive(Debug, Clone, PartialEq)]
pub enum Probe {
    Missing,
    Unreadable(String),
    OtherVersion(i64),
    Ready,
}

/// The search over stored vectors: the replacement point for another vector store.
pub trait VectorIndex {
    /// The `k` most similar vectors of the embedder `key` among the blocks in `candidates` (and only those: the filters
    /// have been applied before), best first, as (chunk id, cosine similarity). Vectors are unit length.
    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>>;
}

/// One embedding configuration's share of the store: what a generation of vectors holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Generation {
    pub embedder_key: String,
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dim: usize,
    pub documents: usize,
    pub vectors: usize,
}

/// The backends that exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Sqlite,
    Jsonl,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Sqlite, Kind::Jsonl];

    pub fn parse(s: &str) -> Result<Kind> {
        match s {
            "sqlite" => Ok(Kind::Sqlite),
            "jsonl" => Ok(Kind::Jsonl),
            other => Err(Error::usage(format!("unknown vector store backend '{other}' (sqlite, jsonl)"))),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Sqlite => "sqlite",
            Kind::Jsonl => "jsonl",
        }
    }

    /// Where this backend keeps its files: its own file or directory, shared with no other backend.
    pub fn location(self, ws: &Workspace) -> std::path::PathBuf {
        match self {
            Kind::Sqlite => ws.index_db_path(),
            Kind::Jsonl => ws.index_jsonl_dir(),
        }
    }

    /// The version of the stored layout of this backend. A store of another version is not converted: it is rebuilt from the cache.
    pub fn layout_version(self) -> i64 {
        STORE_VERSION
    }
}

/// What a backend has to do. The text blocks and the vectors live in it; everything in it is derived from the cache and the inventory
/// and can be rebuilt. `nearest` only ranks blocks it is given (the filters have been applied before).
pub trait Backend {
    fn kind(&self) -> Kind;
    fn doc_rows(&self) -> Result<BTreeMap<String, DocRow>>;
    /// Replaces everything stored about a document (its blocks, and any vectors made from the old blocks) by `chunks`.
    fn replace_document(&mut self, row: &DocRow, chunks: &[ChunkRecord]) -> Result<()>;
    fn remove_document(&mut self, doc: &str) -> Result<()>;
    fn chunks_of(&self, doc: &str) -> Result<Vec<ChunkRecord>>;
    fn chunk(&self, chunk_id: &str) -> Result<Option<ChunkRecord>>;
    fn embedders(&self) -> Result<Vec<EmbedderRow>>;
    /// Registers the embedding configuration vectors are about to be made with. Vectors of other configurations are kept apart.
    fn use_embedder(&mut self, e: &EmbedderRow) -> Result<()>;
    /// Removes the vectors of every embedding configuration but `key`; returns how many configurations were removed.
    fn drop_embedders_except(&mut self, key: &str) -> Result<usize>;
    fn vec_row(&self, doc: &str, key: &str) -> Result<Option<VecRow>>;
    fn replace_doc_vectors(&mut self, row: &VecRow, vectors: &[(String, Vec<f32>)]) -> Result<()>;
    fn vector_of(&self, chunk_id: &str, key: &str) -> Result<Option<Vec<f32>>>;
    /// How many vectors of `key` are really stored for a document (a damaged store can hold fewer than its record says).
    fn vector_count(&self, doc: &str, key: &str) -> Result<usize>;
    fn generations(&self) -> Result<Vec<Generation>>;
    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>>;
}

fn rank(mut scored: Vec<(String, f32)>, k: usize) -> Vec<(String, f32)> {
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

// ---- sqlite ----------------------------------------------------------------------------------------------------------------

struct Sqlite {
    conn: Connection,
}

fn db(e: rusqlite::Error) -> Error {
    Error::failed(format!("the index store: {e}"))
}

fn blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn unblob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn remove_doc(tx: &rusqlite::Transaction<'_>, doc: &str) -> Result<()> {
    tx.execute("DELETE FROM vectors WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE document_id = ?1)", [doc]).map_err(db)?;
    tx.execute("DELETE FROM doc_vectors WHERE document_id = ?1", [doc]).map_err(db)?;
    tx.execute("DELETE FROM chunks WHERE document_id = ?1", [doc]).map_err(db)?;
    tx.execute("DELETE FROM documents WHERE document_id = ?1", [doc]).map_err(db)?;
    Ok(())
}

impl Sqlite {
    fn probe(path: &std::path::Path) -> Probe {
        if !path.is_file() {
            return Probe::Missing;
        }
        let check = || -> rusqlite::Result<(i64, String)> {
            let c = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            let v: i64 = c.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let q: String = c.query_row("PRAGMA quick_check", [], |r| r.get(0))?;
            Ok((v, q))
        };
        match check() {
            Ok((v, q)) if q == "ok" && v == STORE_VERSION => Probe::Ready,
            Ok((v, q)) if q == "ok" => Probe::OtherVersion(v),
            Ok((_, q)) => Probe::Unreadable(format!("the database is damaged ({q})")),
            Err(e) => Probe::Unreadable(e.to_string()),
        }
    }

    fn open_read(path: &std::path::Path) -> Option<Sqlite> {
        Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok().map(|conn| Sqlite { conn })
    }

    fn open_write(path: &std::path::Path) -> Result<Sqlite> {
        let conn = Connection::open(path).map_err(|e| Error::output(format!("{}: {e}", path.display())))?;
        conn.execute_batch(
            "PRAGMA journal_mode = DELETE;
             CREATE TABLE IF NOT EXISTS documents(document_id TEXT PRIMARY KEY, sha256 TEXT NOT NULL, chunker TEXT NOT NULL, inv_sig TEXT NOT NULL, chunks INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS chunks(chunk_id TEXT PRIMARY KEY, document_id TEXT NOT NULL, document_version TEXT NOT NULL, level TEXT NOT NULL, applies_to TEXT NOT NULL,
                 page INTEGER NOT NULL, ordinal INTEGER NOT NULL, section TEXT, text TEXT NOT NULL, text_sha256 TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS chunks_doc ON chunks(document_id, page, ordinal);
             CREATE TABLE IF NOT EXISTS embedders(embedder_key TEXT PRIMARY KEY, provider TEXT NOT NULL, model TEXT NOT NULL, version TEXT NOT NULL, dim INTEGER NOT NULL, config_sha256 TEXT NOT NULL, remote INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS doc_vectors(document_id TEXT NOT NULL, embedder_key TEXT NOT NULL, sha256 TEXT NOT NULL, chunker TEXT NOT NULL, vectors INTEGER NOT NULL, PRIMARY KEY(document_id, embedder_key));
             CREATE TABLE IF NOT EXISTS vectors(chunk_id TEXT NOT NULL, embedder_key TEXT NOT NULL, vec BLOB NOT NULL, PRIMARY KEY(chunk_id, embedder_key));",
        )
        .map_err(db)?;
        conn.pragma_update(None, "user_version", STORE_VERSION).map_err(db)?;
        Ok(Sqlite { conn })
    }

    fn record(r: &rusqlite::Row<'_>) -> rusqlite::Result<ChunkRecord> {
        let applies: String = r.get(4)?;
        Ok(ChunkRecord {
            chunk_id: r.get(0)?,
            document_id: r.get(1)?,
            document_version: r.get(2)?,
            level: r.get(3)?,
            applies_to: serde_json::from_str(&applies).unwrap_or_default(),
            page: r.get::<_, i64>(5)? as u32,
            ordinal: r.get::<_, i64>(6)? as u32,
            section: r.get(7)?,
            text: r.get(8)?,
            text_sha256: r.get(9)?,
        })
    }

    const COLS: &'static str = "chunk_id, document_id, document_version, level, applies_to, page, ordinal, section, text, text_sha256";
}

impl Backend for Sqlite {
    fn kind(&self) -> Kind {
        Kind::Sqlite
    }

    fn doc_rows(&self) -> Result<BTreeMap<String, DocRow>> {
        let mut st = self.conn.prepare("SELECT document_id, sha256, chunker, inv_sig, chunks FROM documents").map_err(db)?;
        let rows = st.query_map([], |r| Ok(DocRow { document_id: r.get(0)?, sha256: r.get(1)?, chunker: r.get(2)?, inv_sig: r.get(3)?, chunks: r.get::<_, i64>(4)? as usize })).map_err(db)?;
        let mut m = BTreeMap::new();
        for r in rows {
            let r = r.map_err(db)?;
            m.insert(r.document_id.clone(), r);
        }
        Ok(m)
    }

    fn replace_document(&mut self, row: &DocRow, chunks: &[ChunkRecord]) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        remove_doc(&tx, &row.document_id)?;
        for c in chunks {
            tx.execute(
                "INSERT INTO chunks(chunk_id, document_id, document_version, level, applies_to, page, ordinal, section, text, text_sha256) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![c.chunk_id, c.document_id, c.document_version, c.level, serde_json::to_string(&c.applies_to).expect("serialises"), c.page, c.ordinal, c.section, c.text, c.text_sha256],
            )
            .map_err(db)?;
        }
        tx.execute("INSERT INTO documents(document_id, sha256, chunker, inv_sig, chunks) VALUES (?1,?2,?3,?4,?5)", params![row.document_id, row.sha256, row.chunker, row.inv_sig, row.chunks as i64]).map_err(db)?;
        tx.commit().map_err(db)
    }

    fn remove_document(&mut self, doc: &str) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        remove_doc(&tx, doc)?;
        tx.commit().map_err(db)
    }

    fn chunks_of(&self, doc: &str) -> Result<Vec<ChunkRecord>> {
        let mut st = self.conn.prepare(&format!("SELECT {} FROM chunks WHERE document_id = ?1 ORDER BY page, ordinal", Sqlite::COLS)).map_err(db)?;
        let rows = st.query_map([doc], Sqlite::record).map_err(db)?;
        rows.map(|r| r.map_err(db)).collect()
    }

    fn chunk(&self, chunk_id: &str) -> Result<Option<ChunkRecord>> {
        self.conn.query_row(&format!("SELECT {} FROM chunks WHERE chunk_id = ?1", Sqlite::COLS), [chunk_id], Sqlite::record).optional().map_err(db)
    }

    fn embedders(&self) -> Result<Vec<EmbedderRow>> {
        let mut st = self.conn.prepare("SELECT embedder_key, provider, model, version, dim, config_sha256, remote FROM embedders ORDER BY embedder_key").map_err(db)?;
        let rows = st
            .query_map([], |r| Ok(EmbedderRow { embedder_key: r.get(0)?, provider: r.get(1)?, model: r.get(2)?, version: r.get(3)?, dim: r.get::<_, i64>(4)? as usize, config_sha256: r.get(5)?, remote: r.get::<_, i64>(6)? != 0 }))
            .map_err(db)?;
        rows.map(|r| r.map_err(db)).collect()
    }

    fn use_embedder(&mut self, e: &EmbedderRow) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO embedders(embedder_key, provider, model, version, dim, config_sha256, remote) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![e.embedder_key, e.provider, e.model, e.version, e.dim as i64, e.config_sha256, e.remote as i64],
            )
            .map_err(db)?;
        Ok(())
    }

    fn drop_embedders_except(&mut self, key: &str) -> Result<usize> {
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute("DELETE FROM vectors WHERE embedder_key <> ?1", [key]).map_err(db)?;
        tx.execute("DELETE FROM doc_vectors WHERE embedder_key <> ?1", [key]).map_err(db)?;
        let n = tx.execute("DELETE FROM embedders WHERE embedder_key <> ?1", [key]).map_err(db)?;
        tx.commit().map_err(db)?;
        Ok(n)
    }

    fn vec_row(&self, doc: &str, key: &str) -> Result<Option<VecRow>> {
        self.conn
            .query_row("SELECT document_id, embedder_key, sha256, chunker, vectors FROM doc_vectors WHERE document_id = ?1 AND embedder_key = ?2", params![doc, key], |r| {
                Ok(VecRow { document_id: r.get(0)?, embedder_key: r.get(1)?, sha256: r.get(2)?, chunker: r.get(3)?, vectors: r.get::<_, i64>(4)? as usize })
            })
            .optional()
            .map_err(db)
    }

    fn replace_doc_vectors(&mut self, row: &VecRow, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute("DELETE FROM vectors WHERE embedder_key = ?1 AND chunk_id IN (SELECT chunk_id FROM chunks WHERE document_id = ?2)", params![row.embedder_key, row.document_id]).map_err(db)?;
        tx.execute("DELETE FROM doc_vectors WHERE document_id = ?1 AND embedder_key = ?2", params![row.document_id, row.embedder_key]).map_err(db)?;
        for (id, v) in vectors {
            tx.execute("INSERT INTO vectors(chunk_id, embedder_key, vec) VALUES (?1,?2,?3)", params![id, row.embedder_key, blob(v)]).map_err(db)?;
        }
        tx.execute("INSERT INTO doc_vectors(document_id, embedder_key, sha256, chunker, vectors) VALUES (?1,?2,?3,?4,?5)", params![row.document_id, row.embedder_key, row.sha256, row.chunker, row.vectors as i64]).map_err(db)?;
        tx.commit().map_err(db)
    }

    fn vector_of(&self, chunk_id: &str, key: &str) -> Result<Option<Vec<f32>>> {
        let b: Option<Vec<u8>> = self.conn.query_row("SELECT vec FROM vectors WHERE chunk_id = ?1 AND embedder_key = ?2", params![chunk_id, key], |r| r.get(0)).optional().map_err(db)?;
        Ok(b.map(|b| unblob(&b)))
    }

    fn vector_count(&self, doc: &str, key: &str) -> Result<usize> {
        self.conn
            .query_row("SELECT count(*) FROM vectors WHERE embedder_key = ?1 AND chunk_id IN (SELECT chunk_id FROM chunks WHERE document_id = ?2)", params![key, doc], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .map_err(db)
    }

    fn generations(&self) -> Result<Vec<Generation>> {
        let mut out = Vec::new();
        for e in self.embedders()? {
            let documents: i64 = self.conn.query_row("SELECT count(*) FROM doc_vectors WHERE embedder_key = ?1", [&e.embedder_key], |r| r.get(0)).map_err(db)?;
            let vectors: i64 = self.conn.query_row("SELECT count(*) FROM vectors WHERE embedder_key = ?1", [&e.embedder_key], |r| r.get(0)).map_err(db)?;
            out.push(Generation { embedder_key: e.embedder_key, provider: e.provider, model: e.model, version: e.version, dim: e.dim, documents: documents as usize, vectors: vectors as usize });
        }
        Ok(out)
    }

    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>> {
        let mut st = self.conn.prepare("SELECT chunk_id, vec FROM vectors WHERE embedder_key = ?1").map_err(db)?;
        let rows = st.query_map([key], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))).map_err(db)?;
        let mut scored: Vec<(String, f32)> = Vec::new();
        for r in rows {
            let (id, b) = r.map_err(db)?;
            if !candidates.contains(&id) {
                continue;
            }
            let v = unblob(&b);
            if v.len() != query.len() {
                continue;
            }
            scored.push((id, v.iter().zip(query).map(|(a, b)| a * b).sum()));
        }
        Ok(rank(scored, k))
    }
}

// ---- jsonl -----------------------------------------------------------------------------------------------------------------

const MANIFEST: &str = "store.json";
const F_DOCS: &str = "documents.jsonl";
const F_CHUNKS: &str = "chunks.jsonl";
const F_EMBEDDERS: &str = "embedders.jsonl";
const F_DOCVEC: &str = "doc_vectors.jsonl";
const F_VECTORS: &str = "vectors.jsonl";
const JSONL_FILES: [&str; 5] = [F_DOCS, F_CHUNKS, F_EMBEDDERS, F_DOCVEC, F_VECTORS];

#[derive(Serialize, Deserialize)]
struct VecLine {
    embedder_key: String,
    chunk_id: String,
    vec: Vec<f32>,
}

struct Jsonl {
    dir: std::path::PathBuf,
    docs: BTreeMap<String, DocRow>,
    chunks: BTreeMap<String, ChunkRecord>,
    embedders: BTreeMap<String, EmbedderRow>,
    doc_vectors: BTreeMap<(String, String), VecRow>,
    /// (embedder key, chunk id) -> vector
    vectors: BTreeMap<(String, String), Vec<f32>>,
    /// manifest: file name -> (sha256, lines)
    files: BTreeMap<String, (String, usize)>,
}

fn jerr(file: &str, e: impl std::fmt::Display) -> String {
    format!("{file}: {e}")
}

fn read_lines<T: for<'de> Deserialize<'de>>(dir: &std::path::Path, name: &str) -> std::result::Result<Vec<T>, String> {
    let text = std::fs::read_to_string(dir.join(name)).map_err(|e| jerr(name, e))?;
    text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()).map(|(i, l)| serde_json::from_str(l).map_err(|e| jerr(name, format!("line {}: {e}", i + 1)))).collect()
}

impl Jsonl {
    fn empty(dir: &std::path::Path) -> Jsonl {
        Jsonl { dir: dir.to_path_buf(), docs: BTreeMap::new(), chunks: BTreeMap::new(), embedders: BTreeMap::new(), doc_vectors: BTreeMap::new(), vectors: BTreeMap::new(), files: BTreeMap::new() }
    }

    fn probe(dir: &std::path::Path) -> Probe {
        let m = dir.join(MANIFEST);
        if !m.is_file() {
            return Probe::Missing;
        }
        let v: serde_json::Value = match std::fs::read_to_string(&m).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str(&t).map_err(|e| e.to_string())) {
            Ok(v) => v,
            Err(e) => return Probe::Unreadable(jerr(MANIFEST, e)),
        };
        if v["format"] != "hcp-manager-vector-store" || v["backend"] != "jsonl" {
            return Probe::Unreadable(format!("{MANIFEST} is not the manifest of a jsonl store"));
        }
        match v["store_version"].as_i64() {
            Some(n) if n == STORE_VERSION => {}
            Some(n) => return Probe::OtherVersion(n),
            None => return Probe::Unreadable(format!("{MANIFEST}: no store_version")),
        }
        // every file is what the manifest says: a half-written or edited file is damage, not data
        for f in JSONL_FILES {
            let want = &v["files"][f];
            let Ok(bytes) = std::fs::read(dir.join(f)) else { return Probe::Unreadable(format!("{f} is missing")) };
            if want["sha256"].as_str() != Some(fr_constraint::hcp::sha256_hex(&bytes).as_str()) {
                return Probe::Unreadable(format!("{f} does not match its checksum in {MANIFEST}"));
            }
        }
        Probe::Ready
    }

    fn load(dir: &std::path::Path) -> std::result::Result<Jsonl, String> {
        let mut s = Jsonl::empty(dir);
        let m: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(MANIFEST)).map_err(|e| jerr(MANIFEST, e))?).map_err(|e| jerr(MANIFEST, e))?;
        for f in JSONL_FILES {
            s.files.insert(f.to_string(), (m["files"][f]["sha256"].as_str().unwrap_or("").to_string(), m["files"][f]["lines"].as_u64().unwrap_or(0) as usize));
        }
        for r in read_lines::<DocRow>(dir, F_DOCS)? {
            s.docs.insert(r.document_id.clone(), r);
        }
        for r in read_lines::<ChunkRecord>(dir, F_CHUNKS)? {
            s.chunks.insert(r.chunk_id.clone(), r);
        }
        for r in read_lines::<EmbedderRow>(dir, F_EMBEDDERS)? {
            s.embedders.insert(r.embedder_key.clone(), r);
        }
        for r in read_lines::<VecRow>(dir, F_DOCVEC)? {
            s.doc_vectors.insert((r.document_id.clone(), r.embedder_key.clone()), r);
        }
        for r in read_lines::<VecLine>(dir, F_VECTORS)? {
            s.vectors.insert((r.embedder_key, r.chunk_id), r.vec);
        }
        Ok(s)
    }

    fn render(&self, name: &str) -> (Vec<u8>, usize) {
        let mut out = Vec::new();
        let mut n = 0;
        let mut push = |v: &dyn erased::Ser| {
            out.extend_from_slice(v.json().as_bytes());
            out.push(b'\n');
            n += 1;
        };
        match name {
            F_DOCS => self.docs.values().for_each(|r| push(&r.clone())),
            F_CHUNKS => {
                let mut cs: Vec<&ChunkRecord> = self.chunks.values().collect();
                cs.sort_by(|a, b| (&a.document_id, a.page, a.ordinal, &a.chunk_id).cmp(&(&b.document_id, b.page, b.ordinal, &b.chunk_id)));
                cs.into_iter().for_each(|r| push(&r.clone()));
            }
            F_EMBEDDERS => self.embedders.values().for_each(|r| push(&r.clone())),
            F_DOCVEC => self.doc_vectors.values().for_each(|r| push(&r.clone())),
            _ => self.vectors.iter().for_each(|((k, id), v)| push(&VecLine { embedder_key: k.clone(), chunk_id: id.clone(), vec: v.clone() })),
        }
        (out, n)
    }

    /// Rewrites the named files (each through a temporary file and a rename) and then the manifest with their checksums.
    fn flush(&mut self, names: &[&str]) -> Result<()> {
        for name in names {
            let (bytes, lines) = self.render(name);
            crate::workspace::write_atomic(&self.dir.join(name), &bytes)?;
            self.files.insert(name.to_string(), (fr_constraint::hcp::sha256_hex(&bytes), lines));
        }
        for f in JSONL_FILES {
            if !self.files.contains_key(f) {
                let (bytes, lines) = self.render(f);
                crate::workspace::write_atomic(&self.dir.join(f), &bytes)?;
                self.files.insert(f.to_string(), (fr_constraint::hcp::sha256_hex(&bytes), lines));
            }
        }
        let files: serde_json::Map<String, serde_json::Value> = self.files.iter().map(|(k, (sha, lines))| (k.clone(), serde_json::json!({"sha256": sha, "lines": lines}))).collect();
        let manifest = serde_json::json!({
            "format": "hcp-manager-vector-store", "backend": "jsonl", "store_version": STORE_VERSION,
            "note": "Derived data: delete this directory and `hcp-manager index build --backend jsonl` makes it again from the cache.",
            "files": files,
        });
        crate::workspace::write_atomic(&self.dir.join(MANIFEST), serde_json::to_string_pretty(&manifest).expect("serialises").as_bytes())
    }

    fn drop_doc(&mut self, doc: &str) {
        let ids: Vec<String> = self.chunks.values().filter(|c| c.document_id == doc).map(|c| c.chunk_id.clone()).collect();
        self.vectors.retain(|(_, id), _| !ids.contains(id));
        self.doc_vectors.retain(|(d, _), _| d != doc);
        self.chunks.retain(|_, c| c.document_id != doc);
        self.docs.remove(doc);
    }
}

mod erased {
    pub trait Ser {
        fn json(&self) -> String;
    }
    impl<T: serde::Serialize> Ser for T {
        fn json(&self) -> String {
            serde_json::to_string(self).expect("serialises")
        }
    }
}

impl Backend for Jsonl {
    fn kind(&self) -> Kind {
        Kind::Jsonl
    }

    fn doc_rows(&self) -> Result<BTreeMap<String, DocRow>> {
        Ok(self.docs.clone())
    }

    fn replace_document(&mut self, row: &DocRow, chunks: &[ChunkRecord]) -> Result<()> {
        self.drop_doc(&row.document_id);
        for c in chunks {
            self.chunks.insert(c.chunk_id.clone(), c.clone());
        }
        self.docs.insert(row.document_id.clone(), row.clone());
        self.flush(&[F_DOCS, F_CHUNKS, F_DOCVEC, F_VECTORS])
    }

    fn remove_document(&mut self, doc: &str) -> Result<()> {
        self.drop_doc(doc);
        self.flush(&[F_DOCS, F_CHUNKS, F_DOCVEC, F_VECTORS])
    }

    fn chunks_of(&self, doc: &str) -> Result<Vec<ChunkRecord>> {
        let mut v: Vec<ChunkRecord> = self.chunks.values().filter(|c| c.document_id == doc).cloned().collect();
        v.sort_by_key(|c| (c.page, c.ordinal));
        Ok(v)
    }

    fn chunk(&self, chunk_id: &str) -> Result<Option<ChunkRecord>> {
        Ok(self.chunks.get(chunk_id).cloned())
    }

    fn embedders(&self) -> Result<Vec<EmbedderRow>> {
        Ok(self.embedders.values().cloned().collect())
    }

    fn use_embedder(&mut self, e: &EmbedderRow) -> Result<()> {
        self.embedders.insert(e.embedder_key.clone(), e.clone());
        self.flush(&[F_EMBEDDERS])
    }

    fn drop_embedders_except(&mut self, key: &str) -> Result<usize> {
        let n = self.embedders.keys().filter(|k| *k != key).count();
        self.embedders.retain(|k, _| k == key);
        self.doc_vectors.retain(|(_, k), _| k == key);
        self.vectors.retain(|(k, _), _| k == key);
        self.flush(&[F_EMBEDDERS, F_DOCVEC, F_VECTORS])?;
        Ok(n)
    }

    fn vec_row(&self, doc: &str, key: &str) -> Result<Option<VecRow>> {
        Ok(self.doc_vectors.get(&(doc.to_string(), key.to_string())).cloned())
    }

    fn replace_doc_vectors(&mut self, row: &VecRow, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        let ids: Vec<String> = self.chunks.values().filter(|c| c.document_id == row.document_id).map(|c| c.chunk_id.clone()).collect();
        self.vectors.retain(|(k, id), _| !(k == &row.embedder_key && ids.contains(id)));
        for (id, v) in vectors {
            self.vectors.insert((row.embedder_key.clone(), id.clone()), v.clone());
        }
        self.doc_vectors.insert((row.document_id.clone(), row.embedder_key.clone()), row.clone());
        self.flush(&[F_DOCVEC, F_VECTORS])
    }

    fn vector_of(&self, chunk_id: &str, key: &str) -> Result<Option<Vec<f32>>> {
        Ok(self.vectors.get(&(key.to_string(), chunk_id.to_string())).cloned())
    }

    fn vector_count(&self, doc: &str, key: &str) -> Result<usize> {
        Ok(self.chunks.values().filter(|c| c.document_id == doc && self.vectors.contains_key(&(key.to_string(), c.chunk_id.clone()))).count())
    }

    fn generations(&self) -> Result<Vec<Generation>> {
        Ok(self
            .embedders
            .values()
            .map(|e| Generation {
                embedder_key: e.embedder_key.clone(),
                provider: e.provider.clone(),
                model: e.model.clone(),
                version: e.version.clone(),
                dim: e.dim,
                documents: self.doc_vectors.keys().filter(|(_, k)| k == &e.embedder_key).count(),
                vectors: self.vectors.keys().filter(|(k, _)| k == &e.embedder_key).count(),
            })
            .collect())
    }

    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>> {
        let scored = self
            .vectors
            .iter()
            .filter(|((ek, id), v)| ek == key && candidates.contains(id) && v.len() == query.len())
            .map(|((_, id), v)| (id.clone(), v.iter().zip(query).map(|(a, b)| a * b).sum::<f32>()))
            .collect();
        Ok(rank(scored, k))
    }
}

// ---- the store the rest of the program uses ---------------------------------------------------------------------------------

/// The derived index, whichever backend keeps it. Every method is the one of [`Backend`].
pub struct Store {
    inner: Box<dyn Backend>,
}

impl Store {
    /// The backend in force: `--backend`, else `store.backend` of embedding.yaml, else sqlite.
    pub fn kind(ws: &Workspace) -> Result<Kind> {
        if let Some(b) = &ws.backend {
            return Kind::parse(b);
        }
        match crate::embed::load_settings(ws) {
            Ok(s) => s.backend.as_deref().map(Kind::parse).unwrap_or(Ok(Kind::Sqlite)),
            Err(_) => Ok(Kind::Sqlite),
        }
    }

    pub fn location(ws: &Workspace) -> std::path::PathBuf {
        Store::kind(ws).unwrap_or(Kind::Sqlite).location(ws)
    }

    pub fn probe(ws: &Workspace) -> Probe {
        match Store::kind(ws).unwrap_or(Kind::Sqlite) {
            Kind::Sqlite => Sqlite::probe(&ws.index_db_path()),
            Kind::Jsonl => Jsonl::probe(&ws.index_jsonl_dir()),
        }
    }

    /// The store for reading, or `None` when there is none that can be read (missing, another version, damaged).
    pub fn open_read(ws: &Workspace) -> Option<Store> {
        if Store::probe(ws) != Probe::Ready {
            return None;
        }
        match Store::kind(ws).ok()? {
            Kind::Sqlite => Sqlite::open_read(&ws.index_db_path()).map(|b| Store { inner: Box::new(b) }),
            Kind::Jsonl => Jsonl::load(&ws.index_jsonl_dir()).ok().map(|b| Store { inner: Box::new(b) }),
        }
    }

    /// The store for writing. One of another version or one that cannot be read is replaced (it holds nothing that the cache cannot
    /// rebuild) and the reason is returned.
    pub fn open_write(ws: &Workspace) -> Result<(Store, Option<String>)> {
        let kind = Store::kind(ws)?;
        let dir = ws.derived_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::output(format!("{}: {e}", dir.display())))?;
        let location = kind.location(ws);
        let probe = Store::probe(ws);
        let mut replaced = None;
        let remove = |p: &std::path::Path| {
            let _ = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        };
        match &probe {
            Probe::Ready | Probe::Missing => {}
            Probe::OtherVersion(v) => {
                replaced = Some(format!("the {} store was layout version {v}, this program writes {STORE_VERSION}: made again", kind.name()));
                remove(&location);
            }
            Probe::Unreadable(e) => {
                replaced = Some(format!("the {} store could not be read ({e}): made again", kind.name()));
                remove(&location);
            }
        }
        let inner: Box<dyn Backend> = match kind {
            Kind::Sqlite => Box::new(Sqlite::open_write(&location)?),
            Kind::Jsonl => {
                std::fs::create_dir_all(&location).map_err(|e| Error::output(format!("{}: {e}", location.display())))?;
                let mut j = if matches!(probe, Probe::Ready) { Jsonl::load(&location).map_err(|e| Error::failed(format!("the index store: {e}")))? } else { Jsonl::empty(&location) };
                if !matches!(probe, Probe::Ready) {
                    j.flush(&[])?;
                }
                Box::new(j)
            }
        };
        Ok((Store { inner }, replaced))
    }

    pub fn backend(&self) -> Kind {
        self.inner.kind()
    }
    pub fn doc_rows(&self) -> Result<BTreeMap<String, DocRow>> {
        self.inner.doc_rows()
    }
    pub fn doc_row(&self, doc: &str) -> Result<Option<DocRow>> {
        Ok(self.inner.doc_rows()?.remove(doc))
    }
    pub fn replace_document(&mut self, row: &DocRow, chunks: &[ChunkRecord]) -> Result<()> {
        self.inner.replace_document(row, chunks)
    }
    pub fn remove_document(&mut self, doc: &str) -> Result<()> {
        self.inner.remove_document(doc)
    }
    pub fn chunks_of(&self, doc: &str) -> Result<Vec<ChunkRecord>> {
        self.inner.chunks_of(doc)
    }
    pub fn chunk(&self, chunk_id: &str) -> Result<Option<ChunkRecord>> {
        self.inner.chunk(chunk_id)
    }
    pub fn embedders(&self) -> Result<Vec<EmbedderRow>> {
        self.inner.embedders()
    }
    pub fn use_embedder(&mut self, e: &EmbedderRow) -> Result<()> {
        self.inner.use_embedder(e)
    }
    pub fn drop_embedders_except(&mut self, key: &str) -> Result<usize> {
        self.inner.drop_embedders_except(key)
    }
    pub fn vec_row(&self, doc: &str, key: &str) -> Result<Option<VecRow>> {
        self.inner.vec_row(doc, key)
    }
    pub fn replace_doc_vectors(&mut self, row: &VecRow, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        self.inner.replace_doc_vectors(row, vectors)
    }
    pub fn vector_of(&self, chunk_id: &str, key: &str) -> Result<Option<Vec<f32>>> {
        self.inner.vector_of(chunk_id, key)
    }
    pub fn vector_count(&self, doc: &str, key: &str) -> Result<usize> {
        self.inner.vector_count(doc, key)
    }
    pub fn generations(&self) -> Result<Vec<Generation>> {
        self.inner.generations()
    }
}

impl VectorIndex for Store {
    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>> {
        self.inner.nearest(key, query, candidates, k)
    }
}
