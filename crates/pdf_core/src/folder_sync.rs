// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Bounded, local-folder import and annotation publication for cloud folder
//! synchronizers. This module never performs network or account operations.

use std::collections::{HashMap, HashSet};
use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::annotations::AnnotationSet;
use crate::engine::mupdf::MupdfEngine;
use crate::engine::{Document, RenderEngine};
use crate::export::export_pdf_annotated;

const MANIFEST_NAME: &str = "folder-library.json";
const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_BACKUP_SCAN_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DOCUMENTS: usize = 10_000;
const MAX_PDF_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_SCAN_ENTRIES: usize = 100_000;
const MAX_SCAN_DEPTH: usize = 64;
const MAX_RELATIVE_PATH_BYTES: usize = 512;
const MAX_FAILED_STAMPS: usize = MAX_DOCUMENTS;
const MAX_IMPORTS_PER_SCAN: usize = 4;
// A full read of a large textbook is costly. Verify one current source per
// day, rotating through imported paths; normal edits are detected immediately
// through size, mtime, ctime, device and inode metadata.
const FULL_HASH_VERIFY_INTERVAL_NS: u128 = 24 * 60 * 60 * 1_000_000_000;
const BACKUP_SCHEMA: u32 = 1;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug)]
pub enum FolderSyncError {
    Io(std::io::Error),
    Invalid(String),
    Pdf(String),
    Json(serde_json::Error),
}

impl std::fmt::Display for FolderSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "folder sync I/O error: {error}"),
            Self::Invalid(message) => write!(f, "invalid folder sync data: {message}"),
            Self::Pdf(message) => write!(f, "invalid PDF: {message}"),
            Self::Json(error) => write!(f, "folder sync JSON error: {error}"),
        }
    }
}

impl std::error::Error for FolderSyncError {}
impl From<std::io::Error> for FolderSyncError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<serde_json::Error> for FolderSyncError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

pub type FolderSyncResult<T> = std::result::Result<T, FolderSyncError>;

/// An immutable local copy of one source version. `id` includes both the
/// relative source path and content digest, so equal PDFs in distinct folders
/// remain distinct library entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedDocument {
    pub id: String,
    pub relative_path: PathBuf,
    pub local_path: PathBuf,
    pub page_count: u32,
    pub source_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Manifest {
    schema: u32,
    documents: Vec<ImportedDocument>,
    publications: HashMap<String, Publication>,
    #[serde(default)]
    source_stamps: HashMap<String, SourceStamp>,
    #[serde(default)]
    failed_stamps: HashMap<String, SourceStamp>,
    #[serde(default)]
    verify_cursor: usize,
    #[serde(default)]
    last_full_verify_ns: Option<u128>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct SourceStamp {
    size: u64,
    modified_ns: Option<u128>,
    changed_ns: Option<i128>,
    device: Option<u64>,
    inode: Option<u64>,
}

impl Default for Manifest {
    fn default() -> Self {
        Self {
            schema: 1,
            documents: Vec::new(),
            publications: HashMap::new(),
            source_stamps: HashMap::new(),
            failed_stamps: HashMap::new(),
            verify_cursor: 0,
            last_full_verify_ns: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Publication {
    pdf_relative: PathBuf,
    backup_relative: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnnotationBackup {
    schema: u32,
    source_hash: String,
    source_path: PathBuf,
    annotations: AnnotationSet,
    #[serde(default)]
    pdf_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirtyPublication {
    backup: AnnotationBackup,
    publication: Publication,
}

/// Local mirror of a source folder, with PDF exports and portable annotation
/// backups written below `<outgoing_root>/_PDFLector`.
pub struct FolderLibrary {
    internal_root: PathBuf,
    incoming_root: PathBuf,
    outgoing_root: PathBuf,
    manifest: Manifest,
    annotations: HashMap<String, AnnotationSet>,
    checked_publication_hashes: HashMap<PathBuf, (SourceStamp, String)>,
    has_more_work: bool,
}

impl FolderLibrary {
    /// Opens or creates the private storage and output folders. The incoming
    /// and outgoing roots may be the same directory.
    pub fn open(
        internal_root: PathBuf,
        incoming_root: PathBuf,
        outgoing_root: PathBuf,
    ) -> FolderSyncResult<Self> {
        fs::create_dir_all(&internal_root)?;
        fs::create_dir_all(&incoming_root)?;
        fs::create_dir_all(&outgoing_root)?;
        // Android exposes some app-private roots through symlinked/bind-mount
        // aliases. Resolve those configured roots once, then reject symlinks
        // introduced below them when creating or traversing output paths.
        let internal_root = fs::canonicalize(internal_root)?;
        let incoming_root = fs::canonicalize(incoming_root)?;
        let outgoing_root = fs::canonicalize(outgoing_root)?;
        let manifest_path = internal_root.join(MANIFEST_NAME);
        let manifest = match fs::metadata(&manifest_path) {
            Ok(meta) => {
                if meta.len() > MAX_MANIFEST_BYTES {
                    return Err(FolderSyncError::Invalid(
                        "manifest exceeds size limit".into(),
                    ));
                }
                let bytes = fs::read(&manifest_path)?;
                let parsed: Manifest = serde_json::from_slice(&bytes)?;
                if parsed.schema != 1 || parsed.documents.len() > MAX_DOCUMENTS {
                    return Err(FolderSyncError::Invalid(
                        "unsupported or oversized manifest".into(),
                    ));
                }
                validate_manifest(&parsed, &internal_root)?;
                parsed
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Manifest::default(),
            Err(error) => return Err(error.into()),
        };
        let mut library = Self {
            internal_root,
            incoming_root,
            outgoing_root,
            manifest,
            annotations: HashMap::new(),
            checked_publication_hashes: HashMap::new(),
            has_more_work: false,
        };
        library.load_backups();
        library.retry_dirty_publications()?;
        Ok(library)
    }

    /// Current known source versions, including versions whose source files
    /// have since been removed or replaced.
    pub fn documents(&self) -> &[ImportedDocument] {
        &self.manifest.documents
    }

    /// Annotations recovered from a matching portable backup, if available.
    pub fn annotations(&self, document_id: &str) -> Option<&AnnotationSet> {
        self.annotations.get(document_id)
    }

    /// Whether another quick scan should be scheduled to continue importing
    /// changed files that remained after the per-scan batch limit.
    pub fn has_more_work(&self) -> bool {
        self.has_more_work
    }

    /// Imports completed PDF files from the incoming tree. Symlinks,
    /// `_PDFLector` trees, oversized files and files MuPDF cannot open are
    /// skipped. Existing imported objects are immutable and never removed.
    pub fn scan(&mut self) -> FolderSyncResult<Vec<ImportedDocument>> {
        let mut paths = Vec::new();
        let mut remaining = MAX_SCAN_ENTRIES;
        collect_pdfs(
            &self.incoming_root,
            &self.incoming_root,
            &mut paths,
            &mut remaining,
            0,
        )?;
        paths.sort();
        let current_source_keys: HashSet<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(&self.incoming_root).ok())
            .filter(|relative| {
                safe_relative(relative)
                    && relative.to_str().is_some()
                    && relative.as_os_str().len() <= MAX_RELATIVE_PATH_BYTES
            })
            .map(|relative| relative.to_string_lossy().into_owned())
            .collect();
        let stamps_before = self.manifest.source_stamps.len() + self.manifest.failed_stamps.len();
        self.manifest
            .source_stamps
            .retain(|key, _| current_source_keys.contains(key));
        self.manifest
            .failed_stamps
            .retain(|key, _| current_source_keys.contains(key));
        if stamps_before != self.manifest.source_stamps.len() + self.manifest.failed_stamps.len() {
            self.save_manifest()?;
        }
        self.verify_one_source_if_due(&paths)?;
        self.has_more_work = false;
        let mut started_imports = 0usize;
        for source in paths.into_iter().take(MAX_DOCUMENTS) {
            let relative = source
                .strip_prefix(&self.incoming_root)
                .map_err(|_| FolderSyncError::Invalid("source escaped incoming root".into()))?
                .to_path_buf();
            if !safe_relative(&relative)
                || relative.to_str().is_none()
                || relative.as_os_str().len() > MAX_RELATIVE_PATH_BYTES
            {
                continue;
            }
            let meta = match fs::symlink_metadata(&source) {
                Ok(meta) if meta.file_type().is_file() => meta,
                _ => continue,
            };
            if meta.len() == 0 || meta.len() > MAX_PDF_BYTES {
                continue;
            }
            let key = relative.to_string_lossy().into_owned();
            let before = source_stamp(&meta);
            let imported_unchanged = before.is_some_and(|stamp| {
                self.manifest.source_stamps.get(&key) == Some(&stamp)
                    && self
                        .manifest
                        .documents
                        .iter()
                        .any(|doc| doc.relative_path == relative && doc.local_path.is_file())
            });
            let known_invalid =
                before.is_some_and(|stamp| self.manifest.failed_stamps.get(&key) == Some(&stamp));
            if imported_unchanged || known_invalid {
                continue;
            }
            if started_imports >= MAX_IMPORTS_PER_SCAN {
                self.has_more_work = true;
                break;
            }
            started_imports += 1;
            let temp = unique_temp_path(&self.internal_root.join("objects"));
            let Some(parent) = temp.parent() else {
                continue;
            };
            fs::create_dir_all(parent)?;
            if copy_bounded(&source, &temp, MAX_PDF_BYTES).is_err() {
                let _ = fs::remove_file(&temp);
                continue;
            }
            let after = fs::symlink_metadata(&source)
                .ok()
                .and_then(|metadata| source_stamp(&metadata));
            if before.is_some() && before != after {
                let _ = fs::remove_file(&temp);
                continue;
            }
            let hash = match hash_file(&temp) {
                Ok(hash) => hash,
                Err(_) => {
                    let _ = fs::remove_file(&temp);
                    continue;
                }
            };
            let page_count = match MupdfEngine::new().and_then(|engine| engine.open(&temp)) {
                Ok(doc) if doc.page_count() > 0 => doc.page_count(),
                _ => {
                    let _ = fs::remove_file(&temp);
                    if before == after
                        && let Some(stamp) = after
                        && (self.manifest.failed_stamps.contains_key(&key)
                            || self.manifest.failed_stamps.len() < MAX_FAILED_STAMPS)
                    {
                        self.manifest.failed_stamps.insert(key, stamp);
                        self.save_manifest()?;
                    }
                    continue;
                }
            };
            let id = document_id(&relative, &hash);
            if let Some(existing) = self.manifest.documents.iter().find(|doc| doc.id == id) {
                let expected = self
                    .internal_root
                    .join("objects")
                    .join(format!("{}.pdf", id));
                if existing.local_path == expected && expected.is_file() {
                    let _ = fs::remove_file(&temp);
                    if let Some(stamp) = after {
                        self.manifest.source_stamps.insert(key, stamp);
                        self.save_manifest()?;
                    }
                    continue;
                }
            }
            let object_dir = self.internal_root.join("objects");
            let final_path = object_dir.join(format!("{id}.pdf"));
            if fs::rename(&temp, &final_path).is_err() {
                let _ = fs::remove_file(&temp);
                if !final_path.exists() {
                    continue;
                }
            }
            let imported = ImportedDocument {
                id,
                relative_path: relative,
                local_path: final_path,
                page_count,
                source_hash: hash,
            };
            if let Some(existing) = self
                .manifest
                .documents
                .iter_mut()
                .find(|doc| doc.id == imported.id)
            {
                *existing = imported;
            } else {
                self.manifest.documents.push(imported);
            }
            if let Some(stamp) = after {
                self.manifest.failed_stamps.remove(&key);
                self.manifest.source_stamps.insert(key, stamp);
            } else {
                self.manifest.source_stamps.remove(&key);
            }
            if self.manifest.documents.len() > MAX_DOCUMENTS {
                self.manifest.documents.pop();
                return Err(FolderSyncError::Invalid("document limit reached".into()));
            }
            self.save_manifest()?;
        }
        self.load_backups();
        Ok(self.manifest.documents.clone())
    }

    /// Publishes a PDF with embedded annotations and a portable JSON backup.
    /// The source is always the immutable original; the output is never used as
    /// the next export base. A durable journal allows interrupted publications
    /// to be retried when this library is reopened.
    pub fn publish(&mut self, pdf_path: &Path, set: &AnnotationSet) -> FolderSyncResult<()> {
        self.publish_with_hook(pdf_path, set, |_, _| Ok(()))
    }

    fn publish_with_hook<F>(
        &mut self,
        pdf_path: &Path,
        set: &AnnotationSet,
        before_install: F,
    ) -> FolderSyncResult<()>
    where
        F: FnOnce(&Path, &Path) -> FolderSyncResult<()>,
    {
        let doc = self
            .manifest
            .documents
            .iter()
            .find(|doc| doc.local_path == pdf_path)
            .cloned()
            .ok_or_else(|| FolderSyncError::Invalid("PDF is not an imported source copy".into()))?;
        let committed = self.manifest.publications.get(&doc.id).cloned();
        let publication = if let Some(committed) = &committed {
            committed.clone()
        } else {
            self.publication_for(&doc)?
        };
        let journal = self
            .internal_root
            .join("dirty")
            .join(format!("{}.json", doc.id));
        atomic_json(
            &journal,
            &DirtyPublication {
                backup: AnnotationBackup {
                    schema: BACKUP_SCHEMA,
                    source_hash: doc.source_hash.clone(),
                    source_path: doc.relative_path.clone(),
                    annotations: set.clone(),
                    pdf_hash: None,
                },
                publication: publication.clone(),
            },
        )?;
        self.write_publication(&doc, &publication, set, committed.is_some(), before_install)?;
        self.manifest
            .publications
            .insert(doc.id.clone(), publication);
        self.save_manifest()?;
        match fs::remove_file(journal) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        self.annotations.insert(doc.id, set.clone());
        Ok(())
    }

    fn publication_for(&self, doc: &ImportedDocument) -> FolderSyncResult<Publication> {
        if let Some(existing) = self.manifest.publications.get(&doc.id) {
            return Ok(existing.clone());
        }
        let base_pdf = doc.relative_path.clone();
        let parent = base_pdf.parent().unwrap_or(Path::new(""));
        let stem = base_pdf
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("document");
        let ext = base_pdf
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("pdf");
        for suffix in 0..1000 {
            let candidate = if suffix == 0 {
                base_pdf.clone()
            } else if suffix == 1 {
                parent.join(format!(
                    "{stem} [PDFLector-{}].{ext}",
                    &doc.source_hash[..12]
                ))
            } else {
                parent.join(format!(
                    "{stem} [PDFLector-{}-{suffix}].{ext}",
                    &doc.source_hash[..12]
                ))
            };
            let backup = PathBuf::from(format!("{}.json", candidate.display()));
            let output_pdf = self
                .outgoing_root
                .join("_PDFLector/Apuntes")
                .join(&candidate);
            let output_backup = self.outgoing_root.join("_PDFLector/Respaldo").join(&backup);
            if !path_exists_including_symlink(&output_pdf)
                && !path_exists_including_symlink(&output_backup)
            {
                return Ok(Publication {
                    pdf_relative: candidate,
                    backup_relative: backup,
                });
            }
        }
        Err(FolderSyncError::Invalid(
            "could not find a free publication filename".into(),
        ))
    }

    fn write_publication(
        &self,
        doc: &ImportedDocument,
        publication: &Publication,
        set: &AnnotationSet,
        allow_replace: bool,
        before_install: impl FnOnce(&Path, &Path) -> FolderSyncResult<()>,
    ) -> FolderSyncResult<()> {
        let pdf_out = self
            .outgoing_root
            .join("_PDFLector/Apuntes")
            .join(&publication.pdf_relative);
        let backup_out = self
            .outgoing_root
            .join("_PDFLector/Respaldo")
            .join(&publication.backup_relative);
        ensure_output_parent(&self.outgoing_root, &pdf_out)?;
        ensure_output_parent(&self.outgoing_root, &backup_out)?;
        let pdf_temp = unique_temp_path(pdf_out.parent().unwrap_or(&self.outgoing_root));
        let backup_temp = unique_temp_path(backup_out.parent().unwrap_or(&self.outgoing_root));
        if let Err(error) = export_pdf_annotated(&doc.local_path, set, &pdf_temp) {
            let _ = fs::remove_file(&pdf_temp);
            return Err(FolderSyncError::Pdf(error.to_string()));
        }
        let pdf_hash = match hash_file(&pdf_temp) {
            Ok(hash) => hash,
            Err(error) => {
                let _ = fs::remove_file(&pdf_temp);
                return Err(error.into());
            }
        };
        let backup = AnnotationBackup {
            schema: BACKUP_SCHEMA,
            source_hash: doc.source_hash.clone(),
            source_path: doc.relative_path.clone(),
            annotations: set.clone(),
            pdf_hash: Some(pdf_hash),
        };
        if let Err(error) = write_json_new(&backup_temp, &backup) {
            let _ = fs::remove_file(&pdf_temp);
            let _ = fs::remove_file(&backup_temp);
            return Err(error);
        }
        if let Err(error) = before_install(&pdf_out, &backup_out) {
            let _ = fs::remove_file(&pdf_temp);
            let _ = fs::remove_file(&backup_temp);
            return Err(error);
        }
        if let Err(error) = install_file(&pdf_temp, &pdf_out, allow_replace) {
            let _ = fs::remove_file(&pdf_temp);
            let _ = fs::remove_file(&backup_temp);
            return Err(error);
        }
        if let Err(error) = install_file(&backup_temp, &backup_out, allow_replace) {
            let _ = fs::remove_file(&backup_temp);
            return Err(error);
        }
        Ok(())
    }

    fn load_backups(&mut self) {
        let backup_root = self.outgoing_root.join("_PDFLector/Respaldo");
        let mut backup_files = Vec::new();
        let mut remaining = MAX_DOCUMENTS;
        let mut remaining_bytes = MAX_BACKUP_SCAN_BYTES;
        let _ = collect_backup_files(
            &backup_root,
            &backup_root,
            &mut backup_files,
            &mut remaining,
            &mut remaining_bytes,
            0,
        );
        backup_files.sort_by_key(|file| std::cmp::Reverse(file.1));
        let docs = self.manifest.documents.clone();
        let mut changed = false;
        for doc in docs {
            let recorded = self
                .manifest
                .publications
                .get(&doc.id)
                .map(|p| &p.backup_relative);
            let matching = backup_files
                .iter()
                .find(|(relative, _, backup)| {
                    backup.schema == BACKUP_SCHEMA
                        && backup.source_hash == doc.source_hash
                        && backup.source_path == doc.relative_path
                        && recorded.is_none_or(|recorded| recorded == relative)
                })
                .or_else(|| {
                    backup_files.iter().find(|(_, _, backup)| {
                        backup.schema == BACKUP_SCHEMA
                            && backup.source_hash == doc.source_hash
                            && backup.source_path == doc.relative_path
                    })
                });
            if let Some((backup_relative, _, backup)) = matching {
                self.annotations
                    .entry(doc.id.clone())
                    .or_insert(backup.annotations.clone());
                let pdf_relative = backup_relative
                    .to_string_lossy()
                    .strip_suffix(".json")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| doc.relative_path.clone());
                let needs_publication_record =
                    self.manifest
                        .publications
                        .get(&doc.id)
                        .is_none_or(|existing| {
                            existing.backup_relative != *backup_relative
                                || existing.pdf_relative != pdf_relative
                        });
                if !needs_publication_record {
                    continue;
                }
                let Some(expected) = backup.pdf_hash.as_ref() else {
                    continue;
                };
                let pdf_path = self
                    .outgoing_root
                    .join("_PDFLector/Apuntes")
                    .join(&pdf_relative);
                let Ok(metadata) = fs::symlink_metadata(&pdf_path) else {
                    self.checked_publication_hashes.remove(&pdf_path);
                    continue;
                };
                if !metadata.file_type().is_file() {
                    self.checked_publication_hashes.remove(&pdf_path);
                    continue;
                }
                let stamp = source_stamp(&metadata);
                let actual = match stamp {
                    Some(stamp)
                        if self
                            .checked_publication_hashes
                            .get(&pdf_path)
                            .is_some_and(|(cached_stamp, _)| *cached_stamp == stamp) =>
                    {
                        self.checked_publication_hashes
                            .get(&pdf_path)
                            .map(|(_, hash)| hash.clone())
                    }
                    _ => match hash_file(&pdf_path) {
                        Ok(hash) => {
                            if let Some(stamp) = stamp {
                                if self.checked_publication_hashes.len() >= MAX_DOCUMENTS {
                                    self.checked_publication_hashes.clear();
                                }
                                self.checked_publication_hashes
                                    .insert(pdf_path.clone(), (stamp, hash.clone()));
                            }
                            Some(hash)
                        }
                        Err(_) => None,
                    },
                };
                if actual.as_deref() == Some(expected.as_str()) {
                    self.checked_publication_hashes.remove(&pdf_path);
                    self.manifest.publications.insert(
                        doc.id.clone(),
                        Publication {
                            pdf_relative,
                            backup_relative: backup_relative.clone(),
                        },
                    );
                    changed = true;
                }
            }
        }
        if changed {
            let _ = self.save_manifest();
        }
    }

    fn retry_dirty_publications(&mut self) -> FolderSyncResult<()> {
        let dirty = self.internal_root.join("dirty");
        let entries = match fs::read_dir(&dirty) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for entry in entries.flatten().take(MAX_DOCUMENTS) {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let meta = match fs::metadata(&path) {
                Ok(m) if m.len() <= MAX_MANIFEST_BYTES => m,
                _ => continue,
            };
            let _ = meta;
            let bytes = match fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let dirty: DirtyPublication = match serde_json::from_slice(&bytes) {
                Ok(dirty) => dirty,
                Err(_) => continue,
            };
            let backup = dirty.backup;
            let Some(doc) = self
                .manifest
                .documents
                .iter()
                .find(|d| {
                    d.source_hash == backup.source_hash && d.relative_path == backup.source_path
                })
                .cloned()
            else {
                continue;
            };
            let committed = self.manifest.publications.get(&doc.id).cloned();
            let mut publication = if let Some(committed) = &committed {
                committed.clone()
            } else if self.publication_outputs_exist(&dirty.publication) {
                self.publication_for(&doc)?
            } else {
                dirty.publication.clone()
            };
            let journal = DirtyPublication {
                backup: backup.clone(),
                publication: publication.clone(),
            };
            atomic_json(&path, &journal)?;
            let mut published = false;
            for _ in 0..3 {
                match self.write_publication(
                    &doc,
                    &publication,
                    &backup.annotations,
                    committed.is_some(),
                    |_, _| Ok(()),
                ) {
                    Ok(()) => {
                        published = true;
                        break;
                    }
                    Err(_) if committed.is_none() => {
                        publication = self.publication_for(&doc)?;
                        atomic_json(
                            &path,
                            &DirtyPublication {
                                backup: backup.clone(),
                                publication: publication.clone(),
                            },
                        )?;
                    }
                    Err(_) => break,
                }
            }
            if published {
                self.manifest
                    .publications
                    .insert(doc.id.clone(), publication);
                self.save_manifest()?;
                self.annotations.insert(doc.id.clone(), backup.annotations);
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    fn save_manifest(&self) -> FolderSyncResult<()> {
        atomic_json(&self.internal_root.join(MANIFEST_NAME), &self.manifest)
    }

    fn publication_outputs_exist(&self, publication: &Publication) -> bool {
        let pdf = self
            .outgoing_root
            .join("_PDFLector/Apuntes")
            .join(&publication.pdf_relative);
        let backup = self
            .outgoing_root
            .join("_PDFLector/Respaldo")
            .join(&publication.backup_relative);
        path_exists_including_symlink(&pdf) || path_exists_including_symlink(&backup)
    }

    fn verify_one_source_if_due(&mut self, source_paths: &[PathBuf]) -> FolderSyncResult<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|duration| duration.as_nanos());
        let Some(now) = now else { return Ok(()) };
        if self
            .manifest
            .last_full_verify_ns
            .is_some_and(|last| now.saturating_sub(last) < FULL_HASH_VERIFY_INTERVAL_NS)
        {
            return Ok(());
        }
        let mut latest_by_path = HashMap::new();
        for doc in &self.manifest.documents {
            latest_by_path.insert(doc.relative_path.clone(), doc.clone());
        }
        let candidates: Vec<ImportedDocument> = source_paths
            .iter()
            .filter_map(|source| source.strip_prefix(&self.incoming_root).ok())
            .filter_map(|relative| latest_by_path.get(relative).cloned())
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let index = self.manifest.verify_cursor % candidates.len();
        let doc = candidates[index].clone();
        let path = self.incoming_root.join(&doc.relative_path);
        if let Ok(metadata) = fs::symlink_metadata(&path)
            && metadata.file_type().is_file()
            && let Ok(hash) = hash_file(&path)
            && hash != doc.source_hash
        {
            self.manifest
                .source_stamps
                .remove(&doc.relative_path.to_string_lossy().into_owned());
        }
        self.manifest.verify_cursor = self.manifest.verify_cursor.wrapping_add(1);
        self.manifest.last_full_verify_ns = Some(now);
        self.save_manifest()?;
        Ok(())
    }
}

fn validate_manifest(manifest: &Manifest, root: &Path) -> FolderSyncResult<()> {
    for doc in &manifest.documents {
        if !safe_relative(&doc.relative_path)
            || doc.relative_path.to_str().is_none()
            || !valid_hash(&doc.source_hash)
            || doc.id != document_id(&doc.relative_path, &doc.source_hash)
        {
            return Err(FolderSyncError::Invalid(
                "manifest contains an unsafe document identity".into(),
            ));
        }
        let expected = root.join("objects").join(format!("{}.pdf", doc.id));
        if doc.local_path != expected {
            return Err(FolderSyncError::Invalid(
                "manifest object path escaped internal root".into(),
            ));
        }
    }
    for publication in manifest.publications.values() {
        if !safe_relative(&publication.pdf_relative) || !safe_relative(&publication.backup_relative)
        {
            return Err(FolderSyncError::Invalid(
                "manifest contains unsafe publication path".into(),
            ));
        }
    }
    Ok(())
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn document_id(relative: &Path, hash: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(path_bytes(relative));
    digest.update([0]);
    digest.update(hash.as_bytes());
    hex_lower(&digest.finalize())
}

fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().as_bytes().to_vec()
}

fn safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn collect_pdfs(
    root: &Path,
    current: &Path,
    out: &mut Vec<PathBuf>,
    remaining_entries: &mut usize,
    depth: usize,
) -> FolderSyncResult<()> {
    if out.len() >= MAX_DOCUMENTS || *remaining_entries == 0 || depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_DOCUMENTS || *remaining_entries == 0 {
            break;
        }
        *remaining_entries -= 1;
        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.is_dir() {
            if entry.file_name() == "_PDFLector" {
                continue;
            }
            collect_pdfs(root, &path, out, remaining_entries, depth + 1)?;
        } else if meta.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf"))
            && path.strip_prefix(root).is_ok_and(safe_relative)
        {
            out.push(path);
        }
    }
    Ok(())
}

fn source_stamp(metadata: &fs::Metadata) -> Option<SourceStamp> {
    let modified_ns = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_nanos());
    #[cfg(unix)]
    let (changed_ns, device, inode) = {
        use std::os::unix::fs::MetadataExt;
        (
            Some(metadata.ctime() as i128 * 1_000_000_000 + metadata.ctime_nsec() as i128),
            Some(metadata.dev()),
            Some(metadata.ino()),
        )
    };
    #[cfg(not(unix))]
    let (changed_ns, device, inode) = (None, None, None);
    Some(SourceStamp {
        size: metadata.len(),
        modified_ns,
        changed_ns,
        device,
        inode,
    })
}

fn collect_backup_files(
    root: &Path,
    current: &Path,
    out: &mut Vec<(PathBuf, Option<u128>, AnnotationBackup)>,
    remaining: &mut usize,
    remaining_bytes: &mut u64,
    depth: usize,
) -> FolderSyncResult<()> {
    if *remaining == 0 || *remaining_bytes == 0 || depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let entries = match fs::read_dir(current) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries.flatten() {
        if *remaining == 0 || *remaining_bytes == 0 {
            break;
        }
        *remaining -= 1;
        let path = entry.path();
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) if !metadata.file_type().is_symlink() => metadata,
            _ => continue,
        };
        if metadata.is_dir() {
            collect_backup_files(root, &path, out, remaining, remaining_bytes, depth + 1)?;
            continue;
        }
        if !metadata.is_file()
            || metadata.len() > *remaining_bytes
            || !path
                .extension()
                .is_some_and(|extension| extension == "json")
        {
            continue;
        }
        let relative = match path.strip_prefix(root) {
            Ok(relative) if safe_relative(relative) => relative.to_path_buf(),
            _ => continue,
        };
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(_) => continue,
        };
        *remaining_bytes = remaining_bytes.saturating_sub(bytes.len() as u64);
        let backup = match serde_json::from_slice::<AnnotationBackup>(&bytes) {
            Ok(backup) => backup,
            Err(_) => continue,
        };
        let modified_ns = metadata
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_nanos());
        out.push((relative, modified_ns, backup));
    }
    Ok(())
}

fn copy_bounded(source: &Path, target: &Path, max: u64) -> std::io::Result<()> {
    let mut input = File::open(source)?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut total = 0u64;
    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count as u64);
        if total > max {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "PDF exceeds size limit",
            ));
        }
        output.write_all(&buffer[..count])?;
    }
    output.sync_all()
}

#[cfg(test)]
std::thread_local! {
    static TEST_HASH_FILE_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn hash_file(path: &Path) -> std::io::Result<String> {
    #[cfg(test)]
    TEST_HASH_FILE_CALLS.with(|count| count.set(count.get() + 1));
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hex_lower(&hash.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 0xf) as usize] as char);
    }
    result
}

fn unique_temp_path(parent: &Path) -> PathBuf {
    let seq = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(".pdflector-{}-{seq}.tmp", std::process::id()))
}

fn ensure_parent(path: &Path) -> FolderSyncResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        // Reject any existing symlink ancestor below output root.
        let mut current = PathBuf::new();
        for component in parent.components() {
            current.push(component.as_os_str());
            if fs::symlink_metadata(&current).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(FolderSyncError::Invalid(
                    "output path crosses a symlink".into(),
                ));
            }
        }
    }
    Ok(())
}

fn ensure_output_parent(root: &Path, output: &Path) -> FolderSyncResult<()> {
    let relative = output.strip_prefix(root).map_err(|_| {
        FolderSyncError::Invalid("publication path escaped configured output root".into())
    })?;
    if !safe_relative(relative) {
        return Err(FolderSyncError::Invalid(
            "publication path is not a safe relative path".into(),
        ));
    }
    let relative_parent = relative.parent().unwrap_or(Path::new(""));
    let mut current = root.to_path_buf();
    for component in relative_parent.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(FolderSyncError::Invalid(
                    "output path crosses a symlink or non-directory".into(),
                ));
            }
            _ => {}
        }
    }
    fs::create_dir_all(output.parent().unwrap_or(root))?;
    let mut current = root.to_path_buf();
    for component in relative_parent.components() {
        current.push(component.as_os_str());
        if fs::symlink_metadata(&current)
            .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir())
        {
            return Err(FolderSyncError::Invalid(
                "output path crosses a symlink or non-directory".into(),
            ));
        }
    }
    Ok(())
}

fn write_json_new<T: Serialize>(path: &Path, value: &T) -> FolderSyncResult<()> {
    let bytes = serde_json::to_vec(value)?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return Err(FolderSyncError::Invalid(
            "JSON file exceeds size limit".into(),
        ));
    }
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}

fn atomic_json<T: Serialize>(path: &Path, value: &T) -> FolderSyncResult<()> {
    ensure_parent(path)?;
    let temp = unique_temp_path(path.parent().unwrap_or(Path::new(".")));
    if let Err(error) = write_json_new(&temp, value) {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    atomic_replace(&temp, path)
}

fn install_file(temp: &Path, target: &Path, allow_replace: bool) -> FolderSyncResult<()> {
    if !allow_replace {
        rename_no_replace(temp, target)?;
    } else {
        atomic_replace(temp, target)?;
    }
    if let Some(parent) = target.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn rename_no_replace(temp: &Path, target: &Path) -> std::io::Result<()> {
    use std::ffi::c_long;
    use std::os::unix::ffi::OsStrExt;

    let temp = CString::new(temp.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    // Android's bionic renameat2 symbol is only available from API 30, while
    // PDFLector supports API 29. The kernel syscall is available on all target
    // kernels, so invoke it through libc's longstanding syscall entry point.
    #[cfg(any(target_arch = "aarch64", target_arch = "riscv64"))]
    const SYS_RENAMEAT2: c_long = 276;
    #[cfg(target_arch = "x86_64")]
    const SYS_RENAMEAT2: c_long = 316;
    #[cfg(target_arch = "x86")]
    const SYS_RENAMEAT2: c_long = 353;
    #[cfg(target_arch = "arm")]
    const SYS_RENAMEAT2: c_long = 382;
    #[cfg(not(any(
        target_arch = "aarch64",
        target_arch = "riscv64",
        target_arch = "x86_64",
        target_arch = "x86",
        target_arch = "arm"
    )))]
    return Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "renameat2 is unsupported on this Linux architecture",
    ));

    unsafe extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }
    // SAFETY: both paths are NUL-terminated C strings, AT_FDCWD selects the
    // current process namespace, and the syscall does not retain either ptr.
    let result = unsafe {
        syscall(
            SYS_RENAMEAT2,
            -100isize,
            temp.as_ptr(),
            -100isize,
            target.as_ptr(),
            1isize,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn rename_no_replace(temp: &Path, target: &Path) -> std::io::Result<()> {
    // Hard links provide atomic no-replace installation on filesystems that
    // support them. Linux/Android use renameat2 above because emulated storage
    // commonly rejects hard links.
    fs::hard_link(temp, target)?;
    fs::remove_file(temp)
}

/// Treat dangling symlinks as occupied names too. `exists()` follows links and
/// reports false for dangling links, which could otherwise be replaced via a
/// publication race.
fn path_exists_including_symlink(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn atomic_replace(temp: &Path, target: &Path) -> FolderSyncResult<()> {
    // rename is atomic on a single filesystem. On Windows replacement may
    // require removing the old target; this product currently targets Android/Linux.
    fs::rename(temp, target)?;
    if let Some(parent) = target.parent()
        && let Ok(dir) = File::open(parent)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod publication_race_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn root() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("pdflector-publication-race-{nonce}"));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn setup(root: &Path) -> (FolderLibrary, ImportedDocument) {
        let incoming = root.join("incoming/course");
        fs::create_dir_all(&incoming).unwrap();
        fs::write(
            incoming.join("book.pdf"),
            include_bytes!("../tests/assets/simple.pdf"),
        )
        .unwrap();
        let mut library = FolderLibrary::open(
            root.join("internal"),
            root.join("incoming"),
            root.join("outgoing"),
        )
        .unwrap();
        let doc = library.scan().unwrap().remove(0);
        (library, doc)
    }

    fn annotations() -> AnnotationSet {
        let mut set = AnnotationSet::new();
        set.add(
            0,
            crate::annotations::Annotation::TextNote(crate::annotations::TextNote {
                anchor: (10.0, 20.0),
                text: "race test".into(),
            }),
        );
        set
    }

    #[test]
    fn retry_after_pdf_name_race_preserves_competing_pdf() {
        let root = root();
        let (mut library, doc) = setup(&root);
        let set = annotations();
        let competitor = root.join("outgoing/_PDFLector/Apuntes/course/book.pdf");
        let result = library.publish_with_hook(&doc.local_path, &set, |pdf_path, _| {
            fs::write(pdf_path, b"competing PDF").map_err(FolderSyncError::Io)
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&competitor).unwrap(), b"competing PDF");
        assert!(!library.manifest.publications.contains_key(&doc.id));
        drop(library);

        let reopened = FolderLibrary::open(
            root.join("internal"),
            root.join("incoming"),
            root.join("outgoing"),
        )
        .unwrap();
        assert_eq!(fs::read(&competitor).unwrap(), b"competing PDF");
        assert!(reopened.manifest.publications.contains_key(&doc.id));
        assert!(
            root.join("outgoing/_PDFLector/Apuntes/course")
                .read_dir()
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("book [PDFLector-"))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn retry_after_backup_name_race_preserves_competing_backup() {
        let root = root();
        let (mut library, doc) = setup(&root);
        let set = annotations();
        let competitor = root.join("outgoing/_PDFLector/Respaldo/course/book.pdf.json");
        let result = library.publish_with_hook(&doc.local_path, &set, |_, backup_path| {
            fs::write(backup_path, b"competing backup").map_err(FolderSyncError::Io)
        });
        assert!(result.is_err());
        assert_eq!(fs::read(&competitor).unwrap(), b"competing backup");
        assert!(!library.manifest.publications.contains_key(&doc.id));
        drop(library);

        let reopened = FolderLibrary::open(
            root.join("internal"),
            root.join("incoming"),
            root.join("outgoing"),
        )
        .unwrap();
        assert_eq!(fs::read(&competitor).unwrap(), b"competing backup");
        assert!(reopened.manifest.publications.contains_key(&doc.id));
        assert!(
            root.join("outgoing/_PDFLector/Apuntes/course")
                .read_dir()
                .unwrap()
                .any(|entry| entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with("book [PDFLector-"))
        );
        let _ = fs::remove_dir_all(root);
    }

    fn reset_hash_calls() {
        TEST_HASH_FILE_CALLS.with(|count| count.set(0));
    }

    fn hash_calls() -> usize {
        TEST_HASH_FILE_CALLS.with(std::cell::Cell::get)
    }

    #[test]
    fn repeated_scans_do_not_rehash_known_published_pdfs() {
        let root = root();
        let (mut library, doc) = setup(&root);
        let set = annotations();
        library.publish(&doc.local_path, &set).unwrap();

        // Warm once so periodic source verification timestamp is set.
        library.scan().unwrap();

        reset_hash_calls();
        library.scan().unwrap();
        library.scan().unwrap();
        assert_eq!(hash_calls(), 0);
        drop(library);

        reset_hash_calls();
        let mut reopened = FolderLibrary::open(
            root.join("internal"),
            root.join("incoming"),
            root.join("outgoing"),
        )
        .unwrap();
        reopened.scan().unwrap();
        assert_eq!(hash_calls(), 0);
        assert!(reopened.manifest.publications.contains_key(&doc.id));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn unknown_and_mismatched_publication_hashes_only_when_needed() {
        let root = root();
        let (mut library, doc) = setup(&root);
        let set = annotations();
        library.publish(&doc.local_path, &set).unwrap();
        let published_pdf = root.join("outgoing/_PDFLector/Apuntes/course/book.pdf");
        let real_pdf_bytes = fs::read(&published_pdf).unwrap();

        // Simulate missing publication entry + colliding unowned file at Apuntes path.
        library.manifest.publications.clear();
        library.save_manifest().unwrap();
        fs::write(&published_pdf, b"unowned").unwrap();
        // Warm periodic source verify timestamp.
        library.scan().unwrap();

        // Unchanged colliding file should not be re-hashed on subsequent scans.
        reset_hash_calls();
        library.scan().unwrap();
        assert_eq!(hash_calls(), 0);
        assert!(!library.manifest.publications.contains_key(&doc.id));

        // Replacing colliding file with the real published PDF triggers a single hash and records it.
        fs::write(&published_pdf, &real_pdf_bytes).unwrap();
        reset_hash_calls();
        library.scan().unwrap();
        assert_eq!(hash_calls(), 1);
        assert!(library.manifest.publications.contains_key(&doc.id));

        // Now that it is recorded, further scans do not hash it again.
        reset_hash_calls();
        library.scan().unwrap();
        assert_eq!(hash_calls(), 0);
        let _ = fs::remove_dir_all(root);
    }
}
