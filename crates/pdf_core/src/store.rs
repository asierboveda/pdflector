// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! SQLite sidecar persistence for [`AnnotationSet`] (Fase 3, docs/PLAN.md
//! §3.5): one `.db` file per PDF, stored next to the document under
//! `annotations/`, designed for Syncthing (a sync conflict is contained to a
//! single file).
//!
//! # Sidecar path convention
//!
//! [`sidecar_path`] maps `.../library/doc.pdf` to
//! `.../library/annotations/doc.db` (PLAN §3.5 tree: `annotations/<id>.db` —
//! the PDF stem plays the role of `<id>`). The `annotations/` directory is
//! created on first [`AnnotationStore::open`].
//!
//! # Schema
//!
//! ```sql
//! CREATE TABLE annotations (
//!     id       INTEGER PRIMARY KEY,
//!     page_idx INTEGER NOT NULL,
//!     kind     TEXT NOT NULL CHECK (kind IN ('stroke','highlight','textnote')),
//!     payload  TEXT NOT NULL
//! );
//! CREATE TABLE meta (
//!     key   TEXT PRIMARY KEY,
//!     value TEXT NOT NULL
//! );
//! ```
//!
//! `payload` holds the serde_json of the *variant* only (e.g. a
//! [`Stroke`](crate::annotations::Stroke) object for `kind = 'stroke'`), so
//! rows stay small and human-readable. `meta` stores `next_id`, the next id
//! to hand out, so ids are never reused after a reload.
//!
//! # Design decisions
//!
//! - **Rewrite on save**: `save` deletes all rows and re-inserts the whole
//!   set inside one transaction. It is simpler and always consistent (no
//!   partial upserts), and annotation counts are small (hundreds), so the
//!   `O(n)` rewrite is far below the frame budget — saves happen on user
//!   action, never per frame. Upsert-per-id would buy nothing here.
//! - **No `WAL`**: the rollback journal keeps the sidecar a single file;
//!   `WAL` would add `-wal`/`-shm` siblings that complicate Syncthing
//!   (PLAN §3.5).
//! - **`next_id` floor**: on load, `next_id` is raised to `max(id)+1` when
//!   rows exist, so a hand-edited or corrupt sidecar can never make
//!   [`AnnotationSet::add`] reuse a stored id.
//! - **Serde bridge**: [`AnnotationSet`]'s fields are private
//!   (annotations.rs), so save/load enumerate the set through its exact
//!   serde representation (`{"by_page": ..., "next_id": ...}`). If
//!   annotations.rs renames those fields, the two
//!   `serde_json::to_value`/`from_value` bridges here must follow. A public
//!   iterator on [`AnnotationSet`] would remove the coupling.
//!
//! # Dependency justification
//!
//! `rusqlite` is the de-facto Rust binding for `SQLite`: MIT-licensed (free
//! distribution, AGENTS.md §3), maintained, and its `bundled` feature
//! compiles `SQLite` from source — no system library, which is what keeps
//! the sidecar working on Android (Fase 6). Added to `Cargo.toml` by the
//! coordinator.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::json;

use crate::annotations::{Annotated, Annotation, AnnotationSet};

/// Sidecar path for `pdf_path` following PLAN §3.5:
/// `<pdf-dir>/annotations/<pdf-stem>.db`.
///
/// The PDF stem (filename without extension) plays the role of the document
/// `<id>` in the plan's tree. A PDF without an extension maps to its full
/// file name; a path without a parent yields a relative `annotations/...`.
/// Sidecar path for `pdf_path` following PLAN §3.5:
/// `<pdf-dir>/annotations/<pdf-stem>-<hash8>.db`.
///
/// The suffix is a stable FNV-1a hash of the PDF path, so two PDFs with the
/// same stem in different directories (`a/doc.pdf` vs `b/doc.pdf`) no longer
/// collide on the same sidecar (fix 2026-08-23, ADR-007). The hash is
/// deterministic across processes (FNV-1a, no random keys). A PDF without an
/// extension maps to its full file name; a path without a parent yields a
/// relative `annotations/...`.
///
/// For backwards compatibility with pre-hash sidecars (`<stem>.db`), use
/// [`resolve_sidecar`] when *opening* an existing store.
pub fn sidecar_path(pdf_path: &Path) -> PathBuf {
    let dir = pdf_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = pdf_path
        .file_stem()
        .or_else(|| pdf_path.file_name())
        .unwrap_or_else(|| std::ffi::OsStr::new("document"));
    let mut name = stem.to_os_string();
    name.push(format!("-{:08x}.db", stable_path_hash(pdf_path)));
    dir.join("annotations").join(name)
}

fn stable_path_hash(pdf_path: &Path) -> u32 {
    let abs = pdf_path.as_os_str().as_encoded_bytes().to_vec();
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in &abs {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash >> 32) as u32
}

pub fn resolve_sidecar(pdf_path: &Path) -> PathBuf {
    let new = sidecar_path(pdf_path);
    let legacy = legacy_sidecar_path(pdf_path);
    if legacy.is_file() && !new.is_file() {
        legacy
    } else {
        new
    }
}

fn legacy_sidecar_path(pdf_path: &Path) -> PathBuf {
    let dir = pdf_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = pdf_path
        .file_stem()
        .or_else(|| pdf_path.file_name())
        .unwrap_or_else(|| std::ffi::OsStr::new("document"));
    let mut name = stem.to_os_string();
    name.push(".db");
    dir.join("annotations").join(name)
}

/// SQLite sidecar store for one PDF's annotations.
///
/// Holds an open [`rusqlite::Connection`]; `save`/`load` serialize the whole
/// [`AnnotationSet`] through the schema above. `open` is cheap (schema
/// creation is `IF NOT EXISTS`), so callers can open per save or keep one
/// store per document — either works.
pub struct AnnotationStore {
    conn: Connection,
    path: PathBuf,
}

impl AnnotationStore {
    /// Opens (creating if needed) the sidecar database at `db_path` and
    /// ensures the schema exists. The parent directory is created too, so a
    /// fresh library works without a setup step.
    ///
    /// `db_path` is the sidecar itself — derive it from a PDF with
    /// [`sidecar_path`].
    pub fn open(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS annotations (
                 id       INTEGER PRIMARY KEY,
                 page_idx INTEGER NOT NULL,
                 kind     TEXT NOT NULL CHECK (kind IN ('stroke','highlight','textnote')),
                 payload  TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )?;
        Ok(Self {
            conn,
            path: db_path.to_path_buf(),
        })
    }

    /// The sidecar path this store was opened on.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the whole set: all rows are deleted and re-inserted in one
    /// transaction, then `next_id` is stored in `meta`. Idempotent (saving
    /// the same set twice yields the same database contents) and preserves
    /// `next_id` and every stored id.
    pub fn save(&self, set: &AnnotationSet) -> Result<()> {
        // Enumeration bridge: the model's fields are private, so read the
        // set back through its exact serde shape (see module doc).
        let value = serde_json::to_value(set)?;
        let next_id: u64 =
            serde_json::from_value(value.get("next_id").cloned().ok_or_else(|| {
                StoreError::Corrupt("AnnotationSet JSON has no `next_id`".into())
            })?)?;
        let by_page: BTreeMap<usize, Vec<Annotated>> =
            serde_json::from_value(value.get("by_page").cloned().ok_or_else(|| {
                StoreError::Corrupt("AnnotationSet JSON has no `by_page`".into())
            })?)?;

        // Encode before touching the DB: a JSON error must not leave the
        // transaction half-open.
        let mut rows: Vec<(i64, i64, &'static str, String)> = Vec::new();
        for (page_idx, anns) in &by_page {
            for ann in anns {
                let (kind, payload) = encode_kind(&ann.kind)?;
                // ids start at 0 and grow monotonically, so `as i64` cannot
                // overflow in practice (u64 > i64::MAX is unreachable).
                rows.push((ann.id as i64, *page_idx as i64, kind, payload));
            }
        }

        // `Connection::transaction()` needs `&mut`; save takes `&self`, so
        // drive the transaction manually — `Tx` rolls back on drop when
        // commit is not reached (any `?` below aborts the save atomically).
        let tx = Tx::begin(&self.conn)?;
        self.conn.execute("DELETE FROM annotations", params![])?;
        self.conn.execute("DELETE FROM meta", params![])?;
        {
            let mut insert = self.conn.prepare(
                "INSERT INTO annotations (id, page_idx, kind, payload) VALUES (?1, ?2, ?3, ?4)",
            )?;
            for (id, page_idx, kind, payload) in &rows {
                insert.execute(params![id, page_idx, kind, payload])?;
            }
        }
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES ('next_id', ?1)",
            params![next_id.to_string()],
        )?;
        tx.commit()
    }

    /// Reads the whole set back: one row per annotation plus `next_id` from
    /// `meta`. Rows are ordered by `id`, which restores the per-page
    /// insertion (z) order — [`AnnotationSet::add`] assigns ids
    /// monotonically, so ascending id == insertion order. A missing `next_id`
    /// (fresh or hand-edited file) falls back to 0, raised to `max(id)+1`
    /// when rows exist.
    pub fn load(&self) -> Result<AnnotationSet> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, page_idx, kind, payload FROM annotations ORDER BY id")?;
        let rows = stmt.query_map(params![], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;

        let mut by_page: BTreeMap<usize, Vec<Annotated>> = BTreeMap::new();
        let mut max_id: u64 = 0;
        for row in rows {
            let (id, page_idx, kind, payload) = row?;
            if id < 0 || page_idx < 0 {
                return Err(StoreError::Corrupt(format!(
                    "negative id ({id}) or page_idx ({page_idx}) in sidecar"
                )));
            }
            let id = id as u64;
            let page_idx = page_idx as usize;
            max_id = max_id.max(id);
            by_page.entry(page_idx).or_default().push(Annotated {
                id,
                page_idx,
                kind: decode_kind(&kind, &payload)?,
            });
        }

        let stored_next_id: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'next_id'",
                params![],
                |row| row.get(0),
            )
            .optional()?;
        let mut next_id = match stored_next_id {
            Some(raw) => raw.parse::<u64>().map_err(|_| {
                StoreError::Corrupt(format!("meta `next_id` value {raw:?} is not a u64"))
            })?,
            None => 0,
        };
        // Defensive floor: a hand-edited sidecar must never make `add` reuse
        // a stored id. Empty sets keep their stored value (0 for a fresh
        // set), so the empty-set round-trip stays exact.
        if !by_page.is_empty() {
            next_id = next_id.max(max_id.saturating_add(1));
        }

        // Rebuild through the set's serde shape (mirror of `save`).
        let value = json!({ "by_page": by_page, "next_id": next_id });
        let set: AnnotationSet = serde_json::from_value(value)?;
        Ok(set)
    }
}

/// Manual transaction guard for `&self` writers: rusqlite's
/// [`rusqlite::Transaction`] requires `&mut Connection`, which `save` does
/// not have. `BEGIN` on construction, `COMMIT` on [`Tx::commit`], `ROLLBACK`
/// on drop if commit was not reached — any `?` in the surrounding function
/// therefore aborts the write atomically.
struct Tx<'a> {
    conn: &'a Connection,
    done: bool,
}

impl<'a> Tx<'a> {
    fn begin(conn: &'a Connection) -> Result<Self> {
        conn.execute_batch("BEGIN")?;
        Ok(Self { conn, done: false })
    }

    fn commit(mut self) -> Result<()> {
        self.conn.execute_batch("COMMIT")?;
        self.done = true;
        Ok(())
    }
}

impl Drop for Tx<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.conn.execute_batch("ROLLBACK");
        }
    }
}

/// Serializes one variant to its `(kind, payload)` row pair: `payload` is the
/// JSON of the variant only (`Stroke`/`Highlight`/`TextNote`), `kind` the row
/// tag.
fn encode_kind(kind: &Annotation) -> Result<(&'static str, String)> {
    match kind {
        Annotation::Stroke(s) => Ok(("stroke", serde_json::to_string(s)?)),
        Annotation::Highlight(h) => Ok(("highlight", serde_json::to_string(h)?)),
        Annotation::TextNote(n) => Ok(("textnote", serde_json::to_string(n)?)),
    }
}

/// Inverse of [`encode_kind`]: parses `payload` back into the variant named
/// by `kind`. Unknown kinds are a data error (the schema `CHECK` constraint
/// should have rejected them at write time).
fn decode_kind(kind: &str, payload: &str) -> Result<Annotation> {
    match kind {
        "stroke" => Ok(Annotation::Stroke(serde_json::from_str(payload)?)),
        "highlight" => Ok(Annotation::Highlight(serde_json::from_str(payload)?)),
        "textnote" => Ok(Annotation::TextNote(serde_json::from_str(payload)?)),
        other => Err(StoreError::Corrupt(format!(
            "unknown annotation kind {other:?}"
        ))),
    }
}

/// Errors from [`AnnotationStore`] operations.
#[derive(Debug)]
pub enum StoreError {
    /// `SQLite` failure (I/O, SQL, constraints).
    Sqlite(rusqlite::Error),
    /// A payload is not valid JSON for its kind, or the set could not be
    /// rebuilt from the sidecar.
    Json(serde_json::Error),
    /// The sidecar directory could not be created.
    Io(std::io::Error),
    /// The sidecar data violates the schema invariants (unknown `kind`,
    /// negative `id`/`page_idx`, malformed `next_id`).
    Corrupt(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sqlite(e) => write!(f, "sqlite error: {e}"),
            Self::Json(e) => write!(f, "invalid annotation JSON: {e}"),
            Self::Io(e) => write!(f, "sidecar I/O error: {e}"),
            Self::Corrupt(msg) => write!(f, "corrupt sidecar: {msg}"),
        }
    }
}

impl std::error::Error for StoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Sqlite(e) => Some(e),
            Self::Json(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::Corrupt(_) => None,
        }
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Sqlite(err)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Convenience alias for store operations.
pub type Result<T> = std::result::Result<T, StoreError>;
