//! The embedded store: one SQLite file, `derived/index.db`, with the text blocks of the cached documents, their vectors and a record
//! of what each was built from (the digest of the source, the chunking strategy, the entry of the inventory, the embedding
//! configuration). It is derived data: delete it and `index build` makes it again from the cache. There is no server, and nothing
//! in a pack, an IR or a verdict reads it.
//!
//! The vector side is behind the [`VectorIndex`] trait; the flat scan over the blocks that survived the filters (device, version,
//! source level) is the implementation that ships. Another store (an ANN library, a database) implements the same trait.

use crate::error::{Error, Result};
use crate::text::Chunk;
use crate::workspace::Workspace;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};

/// The layout of the tables. A store of another version is not migrated, it is rebuilt.
pub const STORE_VERSION: i64 = 1;

/// How the blocks are cut: paragraph boundaries, 400 to 1500 characters (see `text::chunk_pages`). Part of the record of every
/// document: a block set made with another strategy is stale.
pub const CHUNKER: &str = "paragraph-400-1500-v1";

/// A text block with everything that is kept about it: the document, its version and source level, the devices it is for, the page
/// and section, and the digest of the text. This is the record that `chunk show` prints and that a hit points to.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChunkRecord {
    pub chunk_id: String,
    pub document_id: String,
    pub document_version: String,
    pub level: String,
    /// `PART` or `PART REV`
    pub applies_to: Vec<String>,
    pub page: u32,
    pub ordinal: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
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

#[derive(Debug, Clone, PartialEq)]
pub struct DocRow {
    pub document_id: String,
    pub sha256: String,
    pub chunker: String,
    pub inv_sig: String,
    pub chunks: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EmbedderRow {
    pub embedder_key: String,
    pub provider: String,
    pub model: String,
    pub version: String,
    pub dim: usize,
    pub config_sha256: String,
    pub remote: bool,
}

#[derive(Debug, Clone, PartialEq)]
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

pub struct Store {
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

/// The search over stored vectors: the replacement point for another vector store.
pub trait VectorIndex {
    /// The `k` most similar vectors of the embedder `key` among the blocks in `candidates` (and only those: the filters
    /// have been applied before), best first, as (chunk id, cosine similarity). Vectors are unit length.
    fn nearest(&self, key: &str, query: &[f32], candidates: &HashSet<String>, k: usize) -> Result<Vec<(String, f32)>>;
}

impl Store {
    pub fn probe(ws: &Workspace) -> Probe {
        let p = ws.index_db_path();
        if !p.is_file() {
            return Probe::Missing;
        }
        match Connection::open_with_flags(&p, OpenFlags::SQLITE_OPEN_READ_ONLY).and_then(|c| c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))) {
            Ok(v) if v == STORE_VERSION => Probe::Ready,
            Ok(v) => Probe::OtherVersion(v),
            Err(e) => Probe::Unreadable(e.to_string()),
        }
    }

    /// The store for reading, or `None` when there is none that can be read (missing, another version, damaged).
    pub fn open_read(ws: &Workspace) -> Option<Store> {
        if Store::probe(ws) != Probe::Ready {
            return None;
        }
        Connection::open_with_flags(ws.index_db_path(), OpenFlags::SQLITE_OPEN_READ_ONLY).ok().map(|conn| Store { conn })
    }

    /// The store for writing. A file of another version or one that cannot be read is replaced: it holds nothing that the cache
    /// cannot rebuild. Returns the store and whether it had to be replaced.
    pub fn open_write(ws: &Workspace) -> Result<(Store, Option<String>)> {
        let dir = ws.derived_dir();
        std::fs::create_dir_all(&dir).map_err(|e| Error::output(format!("{}: {e}", dir.display())))?;
        let path = ws.index_db_path();
        let mut replaced = None;
        match Store::probe(ws) {
            Probe::Ready | Probe::Missing => {}
            Probe::OtherVersion(v) => {
                replaced = Some(format!("the store was layout version {v}, this program writes {STORE_VERSION}: made again"));
                let _ = std::fs::remove_file(&path);
            }
            Probe::Unreadable(e) => {
                replaced = Some(format!("the store could not be read ({e}): made again"));
                let _ = std::fs::remove_file(&path);
            }
        }
        let conn = Connection::open(&path).map_err(|e| Error::output(format!("{}: {e}", path.display())))?;
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
        Ok((Store { conn }, replaced))
    }

    pub fn doc_rows(&self) -> Result<BTreeMap<String, DocRow>> {
        let mut st = self.conn.prepare("SELECT document_id, sha256, chunker, inv_sig, chunks FROM documents").map_err(db)?;
        let rows = st
            .query_map([], |r| Ok(DocRow { document_id: r.get(0)?, sha256: r.get(1)?, chunker: r.get(2)?, inv_sig: r.get(3)?, chunks: r.get::<_, i64>(4)? as usize }))
            .map_err(db)?;
        let mut m = BTreeMap::new();
        for r in rows {
            let r = r.map_err(db)?;
            m.insert(r.document_id.clone(), r);
        }
        Ok(m)
    }

    pub fn doc_row(&self, doc: &str) -> Result<Option<DocRow>> {
        Ok(self.doc_rows()?.remove(doc))
    }

    /// Replaces everything stored about a document (its blocks, and any vectors made from the old blocks) by `chunks`.
    pub fn replace_document(&mut self, row: &DocRow, chunks: &[ChunkRecord]) -> Result<()> {
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

    pub fn remove_document(&mut self, doc: &str) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        remove_doc(&tx, doc)?;
        tx.commit().map_err(db)
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

    pub fn chunks_of(&self, doc: &str) -> Result<Vec<ChunkRecord>> {
        let mut st = self.conn.prepare(&format!("SELECT {} FROM chunks WHERE document_id = ?1 ORDER BY page, ordinal", Store::COLS)).map_err(db)?;
        let rows = st.query_map([doc], Store::record).map_err(db)?;
        rows.map(|r| r.map_err(db)).collect()
    }

    pub fn chunk(&self, chunk_id: &str) -> Result<Option<ChunkRecord>> {
        self.conn.query_row(&format!("SELECT {} FROM chunks WHERE chunk_id = ?1", Store::COLS), [chunk_id], Store::record).optional().map_err(db)
    }

    // ---- vectors -------------------------------------------------------------------------------------------------------

    pub fn embedders(&self) -> Result<Vec<EmbedderRow>> {
        let mut st = self.conn.prepare("SELECT embedder_key, provider, model, version, dim, config_sha256, remote FROM embedders ORDER BY embedder_key").map_err(db)?;
        let rows = st
            .query_map([], |r| Ok(EmbedderRow { embedder_key: r.get(0)?, provider: r.get(1)?, model: r.get(2)?, version: r.get(3)?, dim: r.get::<_, i64>(4)? as usize, config_sha256: r.get(5)?, remote: r.get::<_, i64>(6)? != 0 }))
            .map_err(db)?;
        rows.map(|r| r.map_err(db)).collect()
    }

    /// One embedder at a time: vectors of any other embedder are removed, so that vectors of different models never mix.
    pub fn use_embedder(&mut self, e: &EmbedderRow) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute("DELETE FROM vectors WHERE embedder_key <> ?1", [&e.embedder_key]).map_err(db)?;
        tx.execute("DELETE FROM doc_vectors WHERE embedder_key <> ?1", [&e.embedder_key]).map_err(db)?;
        tx.execute("DELETE FROM embedders WHERE embedder_key <> ?1", [&e.embedder_key]).map_err(db)?;
        tx.execute(
            "INSERT OR REPLACE INTO embedders(embedder_key, provider, model, version, dim, config_sha256, remote) VALUES (?1,?2,?3,?4,?5,?6,?7)",
            params![e.embedder_key, e.provider, e.model, e.version, e.dim as i64, e.config_sha256, e.remote as i64],
        )
        .map_err(db)?;
        tx.commit().map_err(db)
    }

    pub fn vec_row(&self, doc: &str, key: &str) -> Result<Option<VecRow>> {
        self.conn
            .query_row("SELECT document_id, embedder_key, sha256, chunker, vectors FROM doc_vectors WHERE document_id = ?1 AND embedder_key = ?2", params![doc, key], |r| {
                Ok(VecRow { document_id: r.get(0)?, embedder_key: r.get(1)?, sha256: r.get(2)?, chunker: r.get(3)?, vectors: r.get::<_, i64>(4)? as usize })
            })
            .optional()
            .map_err(db)
    }

    pub fn replace_doc_vectors(&mut self, row: &VecRow, vectors: &[(String, Vec<f32>)]) -> Result<()> {
        let tx = self.conn.transaction().map_err(db)?;
        tx.execute("DELETE FROM vectors WHERE embedder_key = ?1 AND chunk_id IN (SELECT chunk_id FROM chunks WHERE document_id = ?2)", params![row.embedder_key, row.document_id]).map_err(db)?;
        tx.execute("DELETE FROM doc_vectors WHERE document_id = ?1 AND embedder_key = ?2", params![row.document_id, row.embedder_key]).map_err(db)?;
        for (id, v) in vectors {
            tx.execute("INSERT INTO vectors(chunk_id, embedder_key, vec) VALUES (?1,?2,?3)", params![id, row.embedder_key, blob(v)]).map_err(db)?;
        }
        tx.execute("INSERT INTO doc_vectors(document_id, embedder_key, sha256, chunker, vectors) VALUES (?1,?2,?3,?4,?5)", params![row.document_id, row.embedder_key, row.sha256, row.chunker, row.vectors as i64]).map_err(db)?;
        tx.commit().map_err(db)
    }

    pub fn vector_of(&self, chunk_id: &str, key: &str) -> Result<Option<Vec<f32>>> {
        let b: Option<Vec<u8>> = self.conn.query_row("SELECT vec FROM vectors WHERE chunk_id = ?1 AND embedder_key = ?2", params![chunk_id, key], |r| r.get(0)).optional().map_err(db)?;
        Ok(b.map(|b| unblob(&b)))
    }
}

fn remove_doc(tx: &rusqlite::Transaction<'_>, doc: &str) -> Result<()> {
    tx.execute("DELETE FROM vectors WHERE chunk_id IN (SELECT chunk_id FROM chunks WHERE document_id = ?1)", [doc]).map_err(db)?;
    tx.execute("DELETE FROM doc_vectors WHERE document_id = ?1", [doc]).map_err(db)?;
    tx.execute("DELETE FROM chunks WHERE document_id = ?1", [doc]).map_err(db)?;
    tx.execute("DELETE FROM documents WHERE document_id = ?1", [doc]).map_err(db)?;
    Ok(())
}

impl VectorIndex for Store {
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
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        scored.truncate(k);
        Ok(scored)
    }
}
