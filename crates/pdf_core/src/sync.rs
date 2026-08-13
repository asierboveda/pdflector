// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Syncthing-friendly library layout and local change detection (Fase 4,
//! docs/PLAN.md §3.5).
//!
//! # Layout convention (PLAN §3.5)
//!
//! The library folder is the one Syncthing replicates between devices. The
//! app implements **no network code**: Syncthing does the copying, this
//! module only (a) exposes the sync-friendly paths and (b) watches the local
//! sidecar so the UI can hot-reload annotations when they change on disk.
//!
//! ```text
//! BibliotecaPDF/                  # carpeta que Syncthing sincroniza
//! ├── documento.pdf
//! ├── annotations/
//! │   └── documento.db           # sidecar per PDF (store::sidecar_path)
//! └── library.db                 # library index / reading progress
//! ```
//!
//! [`annotations_dir`] and [`library_index_path`] are the two layout
//! helpers; the per-PDF sidecar itself is [`store::sidecar_path`], reused
//! here so the whole convention lives in one place (store.rs is the source
//! of truth for the `<pdf-dir>/annotations/<stem>.db` mapping).
//!
//! # Change detection
//!
//! [`watch_annotations`] registers an OS watch on the sidecar file **and**
//! on its `annotations/` directory. Both matter: Syncthing replaces the
//! sidecar atomically (write temp + rename), which hands the old path a new
//! inode and invalidates a per-file watch — the directory watch keeps
//! reporting the final path of the rename, so no update is missed. Events
//! are debounced (150 ms of quiet) and coalesced into one `on_change` call
//! per burst. If `annotations/` does not exist yet (never-annotated PDF),
//! the PDF's parent directory is watched as a fallback and `on_change`
//! fires once when the directory appears; the caller creates a fresh
//! watcher each time a document is opened, so this covers the common
//! first-sync case.
//!
//! `on_change` is only a trigger: the caller re-reads the
//! [`AnnotationSet`](crate::annotations::AnnotationSet) from
//! [`AnnotationStore`](crate::store::AnnotationStore). No network I/O
//! happens here.
//!
//! # Dependency justification
//!
//! `notify` (v8) is the de-facto standard for cross-platform filesystem
//! notifications: MIT/Apache-2.0 (free distribution, AGENTS.md §3),
//! maintained, no unsafe. Added to `Cargo.toml` by the coordinator.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::store::sidecar_path;

/// Debounce window: events arriving closer than this are coalesced into one
/// `on_change` call. Syncthing writes small sidecars in bursts of ms, so
/// 150 ms of quiet reliably lands after the atomic rename.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Upper bound of coalesced events per burst, so a continuous stream of
/// changes cannot starve `on_change` forever.
const MAX_COALESCED: usize = 64;

/// `annotations/` directory next to `pdf_path`, per PLAN §3.5.
///
/// Derived from [`store::sidecar_path`] so both stay coherent by
/// construction: the sidecar always lives inside this directory.
pub fn annotations_dir(pdf_path: &Path) -> PathBuf {
    sidecar_path(pdf_path)
        .parent()
        .expect("sidecar always sits in <pdf-dir>/annotations/")
        .to_path_buf()
}

/// `library.db` at the root of the Syncthing-replicated library folder
/// (PLAN §3.5): library index and reading progress. `library_root` is the
/// folder that Syncthing shares between devices.
pub fn library_index_path(library_root: &Path) -> PathBuf {
    library_root.join("library.db")
}

/// Live watcher over one PDF's annotation sidecar.
///
/// Owns the OS watch and the debounce thread; dropping it stops delivering
/// callbacks. See [`watch_annotations`]. Do not drop the watcher from
/// inside its own `on_change` callback (that would join the thread
/// currently running the callback).
pub struct AnnotationWatcher {
    watcher: Option<RecommendedWatcher>,
    debounce: Option<thread::JoinHandle<()>>,
}

/// Registers a watch for changes to the sidecar of `pdf_path` and its
/// `annotations/` directory (see module docs). Every debounced burst of
/// relevant filesystem events calls `on_change` exactly once, from a
/// background thread — never from the caller's thread.
///
/// No initial call is made; if the caller needs the current state it reads
/// the sidecar itself before or after registering.
///
/// # Errors
///
/// - [`SyncError::Notify`]: the OS watcher could not be created or a watch
///   registered (e.g. missing permissions).
/// - [`SyncError::NoWatchTarget`]: neither the sidecar, `annotations/` nor
///   the PDF's parent directory exists (the PDF itself is missing).
pub fn watch_annotations(
    pdf_path: &Path,
    on_change: impl FnMut() + Send + 'static,
) -> Result<AnnotationWatcher> {
    // Absolute paths: notify resolves registered paths against the cwd, so
    // relative inputs would never match the absolute paths in events.
    let pdf = std::path::absolute(pdf_path).unwrap_or_else(|_| pdf_path.to_path_buf());
    let sidecar = sidecar_path(&pdf);
    let ann_dir = sidecar
        .parent()
        .expect("sidecar always sits in <pdf-dir>/annotations/")
        .to_path_buf();

    let (tx, rx) = mpsc::channel::<()>();
    // Clones for the 'static handler; the originals are still needed below
    // to register the watches.
    let watch_sidecar = sidecar.clone();
    let watch_ann_dir = ann_dir.clone();
    let handler = move |res: notify::Result<Event>| {
        // Ignore backend errors (a dropped event is not worth a reload) and
        // forward only events touching the sidecar or its directory.
        if let Ok(event) = res
            && event.paths.iter().any(|p| {
                p == &watch_sidecar || p == &watch_ann_dir || p.starts_with(&watch_ann_dir)
            })
        {
            let _ = tx.send(());
        }
    };
    let mut watcher = RecommendedWatcher::new(handler, Config::default())?;

    // Watch whatever exists; tolerate the first-run state where neither the
    // sidecar nor `annotations/` exists yet.
    let mut watched_any = false;
    if sidecar.exists() {
        watcher.watch(&sidecar, RecursiveMode::NonRecursive)?;
        watched_any = true;
    }
    if ann_dir.exists() {
        watcher.watch(&ann_dir, RecursiveMode::NonRecursive)?;
        watched_any = true;
    } else if let Some(parent) = pdf.parent().filter(|p| p.exists()) {
        // First sync of a never-annotated PDF: watch the library folder so
        // the appearance of `annotations/` still triggers one reload.
        watcher.watch(parent, RecursiveMode::NonRecursive)?;
        watched_any = true;
    }
    if !watched_any {
        return Err(SyncError::NoWatchTarget(pdf));
    }

    let mut on_change = on_change;
    let debounce = thread::Builder::new()
        .name("pdflector-annotation-watcher".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                // Coalesce the burst: keep draining while events keep
                // arriving, then fire once. Bounded so an endless stream
                // still delivers.
                let mut n = 0;
                while n < MAX_COALESCED && rx.recv_timeout(DEBOUNCE).is_ok() {
                    n += 1;
                }
                on_change();
            }
        })?;

    Ok(AnnotationWatcher {
        watcher: Some(watcher),
        debounce: Some(debounce),
    })
}

impl Drop for AnnotationWatcher {
    fn drop(&mut self) {
        // Dropping the watcher closes the OS watch and the handler's
        // sender; the debounce thread then exits on `recv` error. The
        // watcher must go first, or join would wait on a thread whose
        // sender is still alive.
        drop(self.watcher.take());
        if let Some(handle) = self.debounce.take() {
            let _ = handle.join();
        }
    }
}

/// Errors from [`watch_annotations`].
#[derive(Debug)]
pub enum SyncError {
    /// The OS watcher could not be created or a watch registered.
    Notify(notify::Error),
    /// The debounce thread could not be spawned.
    Io(std::io::Error),
    /// No watchable path exists for the PDF (sidecar, `annotations/` and
    /// the PDF's parent directory are all missing).
    NoWatchTarget(PathBuf),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Notify(e) => write!(f, "filesystem watcher error: {e}"),
            Self::Io(e) => write!(f, "watcher thread error: {e}"),
            Self::NoWatchTarget(p) => write!(
                f,
                "no watch target for {}: neither the sidecar, annotations/ \
                 nor the PDF's parent directory exists",
                p.display()
            ),
        }
    }
}

impl std::error::Error for SyncError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Notify(e) => Some(e),
            Self::Io(e) => Some(e),
            Self::NoWatchTarget(_) => None,
        }
    }
}

impl From<notify::Error> for SyncError {
    fn from(err: notify::Error) -> Self {
        Self::Notify(err)
    }
}

impl From<std::io::Error> for SyncError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

/// Convenience alias for sync operations.
pub type Result<T> = std::result::Result<T, SyncError>;
