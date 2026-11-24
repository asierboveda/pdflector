// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Folder synchronization configuration and the ordered annotation actor.

use pdf_core::AnnotationSet;
use pdf_core::folder_sync::{FolderLibrary, ImportedDocument};
use pdf_core::store::AnnotationStore;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

const MAX_PENDING_DOCUMENTS: usize = 8;
const MAX_DEFERRED_DOCUMENTS: usize = 64;
const EVENT_CAPACITY: usize = 4;
const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
const SCAN_INTERVAL: Duration = Duration::from_secs(30);
const RETRY_DELAY: Duration = Duration::from_secs(5);
const EXPORT_IDLE_DELAY: Duration = Duration::from_millis(700);

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct FolderSyncConfig {
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) incoming_root: Option<PathBuf>,
    #[serde(default)]
    pub(crate) outgoing_root: Option<PathBuf>,
}

impl FolderSyncConfig {
    pub(crate) fn is_active(&self) -> bool {
        self.enabled
            && self.incoming_root.as_ref().is_some_and(|p| p.is_absolute())
            && self.outgoing_root.as_ref().is_some_and(|p| p.is_absolute())
    }

    pub(crate) fn load(internal_root: &Path) -> Self {
        let path = internal_root.join("folder_sync.json");
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PendingSave {
    pub(crate) pdf_path: PathBuf,
    pub(crate) sidecar: PathBuf,
    pub(crate) annotations: AnnotationSet,
    #[serde(default)]
    pub(crate) publish: bool,
    #[serde(default)]
    pub(crate) persisted: bool,
}

#[derive(Default)]
struct ActorState {
    pending: VecDeque<PendingSave>,
    in_flight: Option<PendingSave>,
    generation: u64,
    scan_requested: bool,
    flush_requested: bool,
    last_save_enqueued: Option<Instant>,
    stopped: bool,
}

pub(crate) enum FolderSyncEvent {
    Scan(Vec<ImportedDocument>, Vec<(String, AnnotationSet)>),
    Status(String),
}

pub(crate) struct FolderSyncWorker {
    shared: Arc<(Mutex<ActorState>, Condvar)>,
    events: Receiver<FolderSyncEvent>,
    latest_status: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
    available: bool,
}

impl FolderSyncWorker {
    pub(crate) fn start(internal_root: PathBuf, config: FolderSyncConfig) -> Self {
        let shared = Arc::new((Mutex::new(ActorState::default()), Condvar::new()));
        let (event_tx, events) = mpsc::sync_channel(EVENT_CAPACITY);
        let latest_status = Arc::new(Mutex::new(None));
        let actor_status = Arc::clone(&latest_status);
        let actor_shared = Arc::clone(&shared);
        let thread = thread::Builder::new()
            .name("pdflector-folder-sync".into())
            .spawn(move || {
                run_actor(actor_shared, event_tx, actor_status, internal_root, config);
            });
        let available = thread.is_ok();
        Self {
            shared,
            events,
            latest_status,
            thread: thread.ok(),
            available,
        }
    }

    /// Coalesce successive complete snapshots for the same PDF. At most eight
    /// distinct documents can wait behind the single SQLite/export worker.
    pub(crate) fn save(&self, job: PendingSave) -> Result<(), &'static str> {
        if !self.available {
            return Err("annotation worker unavailable");
        }
        let (lock, wake) = &*self.shared;
        let Ok(mut state) = lock.lock() else {
            return Err("annotation worker unavailable");
        };
        if let Some(existing) = state.pending.iter_mut().find(|p| p.sidecar == job.sidecar) {
            let mut job = job;
            job.persisted = false;
            *existing = job;
        } else if state.pending.len()
            + usize::from(
                state
                    .in_flight
                    .as_ref()
                    .is_some_and(|in_flight| in_flight.sidecar != job.sidecar),
            )
            < MAX_PENDING_DOCUMENTS
        {
            state.pending.push_back(job);
        } else {
            return Err("annotation queue full (8 documents)");
        }
        state.generation = state.generation.wrapping_add(1);
        state.last_save_enqueued = Some(Instant::now());
        state.flush_requested = false;
        wake.notify_one();
        Ok(())
    }

    pub(crate) fn flush(&self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.flush_requested = true;
            wake.notify_one();
        }
    }

    pub(crate) fn request_scan(&self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.scan_requested = true;
            wake.notify_one();
        }
    }

    pub(crate) fn try_event(&self) -> Option<FolderSyncEvent> {
        if let Ok(event) = self.events.try_recv() {
            return Some(event);
        }
        if let Ok(mut status) = self.latest_status.lock()
            && let Some(status) = status.take()
        {
            return Some(FolderSyncEvent::Status(status));
        }
        None
    }

    pub(crate) fn latest_snapshot(&self, sidecar: &Path) -> Option<AnnotationSet> {
        let state = self.shared.0.lock().ok()?;
        state
            .pending
            .iter()
            .find(|job| job.sidecar == sidecar)
            .or_else(|| {
                state
                    .in_flight
                    .as_ref()
                    .filter(|job| job.sidecar == sidecar)
            })
            .map(|job| job.annotations.clone())
    }
}

impl Drop for FolderSyncWorker {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut state) = lock.lock() {
            state.stopped = true;
            state.flush_requested = true;
            wake.notify_all();
        }
        if let Some(thread) = self.thread.take() {
            // Do not wait for filesystem work on the lifecycle/UI thread.
            drop(thread);
        }
    }
}

fn run_actor(
    shared: Arc<(Mutex<ActorState>, Condvar)>,
    events: SyncSender<FolderSyncEvent>,
    latest_status: Arc<Mutex<Option<String>>>,
    internal_root: PathBuf,
    config: FolderSyncConfig,
) {
    let journal = internal_root.join("folder_sync").join("pending.json");
    let mut library = open_library(&internal_root, &config, &events, &latest_status);
    match fs::metadata(&journal) {
        Ok(meta) if meta.len() > MAX_JOURNAL_BYTES => emit(
            &events,
            &latest_status,
            FolderSyncEvent::Status("Pending annotation journal exceeds the 64 MiB limit".into()),
        ),
        Ok(_) => match fs::read(&journal) {
            Ok(bytes) => match serde_json::from_slice::<Vec<PendingSave>>(&bytes) {
                Ok(pending) if pending.len() <= MAX_PENDING_DOCUMENTS => {
                    if let Ok(mut state) = shared.0.lock() {
                        for mut recovered in pending {
                            if !config.is_active() {
                                recovered.publish = false;
                                recovered.persisted = false;
                            }
                            if state
                                .pending
                                .iter()
                                .any(|newer| newer.sidecar == recovered.sidecar)
                            {
                                continue;
                            }
                            if state.pending.len() < MAX_PENDING_DOCUMENTS {
                                state.pending.push_back(recovered);
                            } else {
                                emit(&events, &latest_status, FolderSyncEvent::Status(
                                    "Recovered annotation queue is full; pending journal retained".into(),
                                ));
                                break;
                            }
                        }
                    }
                }
                Ok(_) => emit(
                    &events,
                    &latest_status,
                    FolderSyncEvent::Status(
                        "Pending annotation journal exceeds the queue limit".into(),
                    ),
                ),
                Err(error) => emit(
                    &events,
                    &latest_status,
                    FolderSyncEvent::Status(format!("Cannot recover pending annotations: {error}")),
                ),
            },
            Err(error) => emit(
                &events,
                &latest_status,
                FolderSyncEvent::Status(format!("Cannot read pending annotations: {error}")),
            ),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => emit(
            &events,
            &latest_status,
            FolderSyncEvent::Status(format!("Cannot inspect pending annotations: {error}")),
        ),
    }
    let mut retry_at: Option<Instant> = None;
    let mut next_scan = Instant::now();

    loop {
        let (lock, wake) = &*shared;
        let Ok(state) = lock.lock() else { return };
        let mut state = state;
        let export_ready = state.flush_requested
            || state
                .last_save_enqueued
                .is_none_or(|at| at.elapsed() >= EXPORT_IDLE_DELAY);
        let front_is_export = state.pending.front().is_some_and(|job| job.persisted);
        let timeout = next_wake_delay(
            &state,
            export_ready,
            retry_at,
            next_scan,
            config.is_active(),
        );
        let save_waiting = !state.pending.is_empty()
            && ((front_is_export && !export_ready)
                || retry_at.is_some_and(|at| Instant::now() < at));
        if (state.pending.is_empty() || save_waiting) && !state.scan_requested && !state.stopped {
            state = match wake.wait_timeout(state, timeout) {
                Ok((state, _)) => state,
                Err(_) => return,
            };
        }
        if state.stopped && state.pending.is_empty() {
            return;
        }
        let scan_now = config.is_active() && (state.scan_requested || Instant::now() >= next_scan);
        state.scan_requested = false;
        let front_is_export = state.pending.front().is_some_and(|job| job.persisted);
        let idle_ready = state.flush_requested
            || state
                .last_save_enqueued
                .is_none_or(|at| at.elapsed() >= EXPORT_IDLE_DELAY);
        let save_ready = !front_is_export || idle_ready;
        let save = if save_ready && retry_at.is_none_or(|at| Instant::now() >= at) {
            let job = state.pending.pop_front();
            if job.is_some() {
                state.generation = state.generation.wrapping_add(1);
                state.in_flight = job.clone();
            }
            job
        } else {
            None
        };
        drop(state);

        if let Some(job) = save {
            match save_job(&journal, &shared, &mut library, job.clone()) {
                Ok(Some(path)) => {
                    retry_at = None;
                    clear_in_flight(&shared);
                    emit(
                        &events,
                        &latest_status,
                        FolderSyncEvent::Status(format!("Annotations published: {path}")),
                    );
                    clear_flush_if_idle(&shared);
                }
                Ok(None) => {
                    retry_at = None;
                    clear_in_flight(&shared);
                    let status = if job.publish {
                        "Annotations saved locally; publication pending"
                    } else {
                        "Annotations saved"
                    };
                    emit(
                        &events,
                        &latest_status,
                        FolderSyncEvent::Status(status.into()),
                    );
                    clear_flush_if_idle(&shared);
                }
                Err(error) => {
                    if let Ok(mut state) = lock.lock() {
                        state.in_flight = None;
                        requeue_failed_job(&mut state, job);
                        if state.stopped {
                            return;
                        }
                    }
                    emit(&events, &latest_status, FolderSyncEvent::Status(error));
                    retry_at = Some(Instant::now() + RETRY_DELAY);
                }
            }
        }

        if scan_now {
            next_scan = Instant::now() + SCAN_INTERVAL;
            if library.is_none() {
                library = open_library(&internal_root, &config, &events, &latest_status);
            }
            if let Some(folder_library) = library.as_mut() {
                match folder_library.scan() {
                    Ok(documents) => {
                        let more_work = folder_library.has_more_work();
                        let annotations = documents
                            .iter()
                            .filter_map(|doc| {
                                folder_library
                                    .annotations(&doc.id)
                                    .cloned()
                                    .map(|set| (doc.id.clone(), set))
                            })
                            .collect();
                        emit(
                            &events,
                            &latest_status,
                            FolderSyncEvent::Scan(documents, annotations),
                        );
                        if more_work {
                            next_scan = Instant::now() + Duration::from_secs(1);
                        }
                    }
                    Err(error) => emit(
                        &events,
                        &latest_status,
                        FolderSyncEvent::Status(format!("Folder sync scan failed: {error}")),
                    ),
                }
            }
        }
    }
}

fn open_library(
    internal_root: &Path,
    config: &FolderSyncConfig,
    events: &SyncSender<FolderSyncEvent>,
    latest_status: &Mutex<Option<String>>,
) -> Option<FolderLibrary> {
    if !config.is_active() {
        return None;
    }
    match FolderLibrary::open(
        internal_root.join("synced-pdfs"),
        config.incoming_root.clone()?,
        config.outgoing_root.clone()?,
    ) {
        Ok(library) => Some(library),
        Err(error) => {
            emit(
                events,
                latest_status,
                FolderSyncEvent::Status(format!("Folder sync unavailable: {error}")),
            );
            None
        }
    }
}

fn clear_flush_if_idle(shared: &Arc<(Mutex<ActorState>, Condvar)>) {
    if let Ok(mut state) = shared.0.lock()
        && state.pending.is_empty()
    {
        state.flush_requested = false;
    }
}

fn clear_in_flight(shared: &Arc<(Mutex<ActorState>, Condvar)>) {
    if let Ok(mut state) = shared.0.lock() {
        state.in_flight = None;
    }
}

fn requeue_failed_job(state: &mut ActorState, job: PendingSave) {
    if !state
        .pending
        .iter()
        .any(|newer| newer.sidecar == job.sidecar)
    {
        state.pending.push_front(job);
    }
    state.generation = state.generation.wrapping_add(1);
}

fn retain_deferred(
    queue: &mut VecDeque<PendingSave>,
    overflow: &mut Option<PendingSave>,
    job: PendingSave,
) -> bool {
    if let Some(existing) = queue
        .iter_mut()
        .find(|pending| pending.sidecar == job.sidecar)
    {
        *existing = job;
        return true;
    }
    if queue.len() < MAX_DEFERRED_DOCUMENTS {
        queue.push_back(job);
        return true;
    }
    if let Some(existing) = overflow {
        if existing.sidecar == job.sidecar {
            *existing = job;
            return true;
        }
        return false;
    }
    *overflow = Some(job);
    true
}

fn next_wake_delay(
    state: &ActorState,
    export_ready: bool,
    retry_at: Option<Instant>,
    next_scan: Instant,
    scan_enabled: bool,
) -> Duration {
    let save_ready_in = if state.pending.is_empty() {
        Duration::from_secs(24 * 60 * 60)
    } else if state.pending.front().is_some_and(|job| job.persisted) && !export_ready {
        EXPORT_IDLE_DELAY.saturating_sub(
            state
                .last_save_enqueued
                .map(|at| at.elapsed())
                .unwrap_or(EXPORT_IDLE_DELAY),
        )
    } else {
        Duration::ZERO
    };
    let retry_in = retry_at
        .map(|at| at.saturating_duration_since(Instant::now()))
        .unwrap_or(Duration::ZERO);
    let scan_in = if scan_enabled {
        next_scan.saturating_duration_since(Instant::now())
    } else {
        Duration::from_secs(24 * 60 * 60)
    };
    save_ready_in.max(retry_in).min(scan_in)
}

fn save_job(
    journal: &Path,
    shared: &Arc<(Mutex<ActorState>, Condvar)>,
    library: &mut Option<FolderLibrary>,
    job: PendingSave,
) -> Result<Option<String>, String> {
    if job.persisted {
        let folder_library = library
            .as_mut()
            .ok_or_else(|| "Folder sync is unavailable".to_string())?;
        let published_path = folder_library
            .documents()
            .iter()
            .find(|document| document.local_path == job.pdf_path)
            .map(|document| document.relative_path.display().to_string());
        folder_library
            .publish(&job.pdf_path, &job.annotations)
            .map_err(|e| format!("Cannot publish annotated PDF: {e}"))?;
        persist_queue(journal, shared)
            .map_err(|e| format!("Cannot clear annotation journal: {e}"))?;
        return Ok(published_path);
    }

    persist_job_and_queue(journal, shared, &job)
        .map_err(|e| format!("Cannot persist pending annotations: {e}"))?;
    let store =
        AnnotationStore::open(&job.sidecar).map_err(|e| format!("Cannot open annotations: {e}"))?;
    store
        .save(&job.annotations)
        .map_err(|e| format!("Cannot save annotations: {e}"))?;
    if job.publish {
        let mut export = job;
        export.persisted = true;
        let mut state = shared
            .0
            .lock()
            .map_err(|_| "annotation worker unavailable".to_string())?;
        if !state
            .pending
            .iter()
            .any(|newer| newer.sidecar == export.sidecar)
        {
            state.pending.push_front(export);
        }
        state.generation = state.generation.wrapping_add(1);
        drop(state);
    }
    persist_queue(journal, shared).map_err(|e| format!("Cannot update annotation journal: {e}"))?;
    Ok(None)
}

fn persist_job_and_queue(
    journal: &Path,
    shared: &Arc<(Mutex<ActorState>, Condvar)>,
    current: &PendingSave,
) -> std::io::Result<()> {
    loop {
        let (generation, pending) = {
            let state = shared
                .0
                .lock()
                .map_err(|_| std::io::Error::other("annotation worker unavailable"))?;
            let mut pending = vec![current.clone()];
            pending.extend(state.pending.iter().cloned());
            (state.generation, coalesce(pending))
        };
        write_json_atomic(journal, &pending)?;
        if shared
            .0
            .lock()
            .map_err(|_| std::io::Error::other("annotation worker unavailable"))?
            .generation
            == generation
        {
            return Ok(());
        }
    }
}

fn persist_queue(
    journal: &Path,
    shared: &Arc<(Mutex<ActorState>, Condvar)>,
) -> std::io::Result<()> {
    loop {
        let (generation, pending) = {
            let state = shared
                .0
                .lock()
                .map_err(|_| std::io::Error::other("annotation worker unavailable"))?;
            (
                state.generation,
                state.pending.iter().cloned().collect::<Vec<_>>(),
            )
        };
        write_json_atomic(journal, &pending)?;
        if shared
            .0
            .lock()
            .map_err(|_| std::io::Error::other("annotation worker unavailable"))?
            .generation
            == generation
        {
            return Ok(());
        }
    }
}

fn coalesce(pending: Vec<PendingSave>) -> Vec<PendingSave> {
    let mut result: Vec<PendingSave> = Vec::new();
    for job in pending {
        if let Some(existing) = result
            .iter_mut()
            .find(|current| current.sidecar == job.sidecar)
        {
            *existing = job;
        } else {
            result.push(job);
        }
    }
    result
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::other("journal path has no parent"));
    };
    fs::create_dir_all(parent)?;
    let temp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    let mut file = fs::File::create(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(temp, path)?;
    if let Ok(directory) = fs::File::open(parent) {
        directory.sync_all()?;
    }
    Ok(())
}

fn emit(
    events: &SyncSender<FolderSyncEvent>,
    latest_status: &Mutex<Option<String>>,
    event: FolderSyncEvent,
) {
    match events.try_send(event) {
        Ok(()) | Err(TrySendError::Disconnected(_)) => {}
        Err(TrySendError::Full(FolderSyncEvent::Status(status))) => {
            if let Ok(mut latest) = latest_status.lock() {
                *latest = Some(status);
            }
        }
        Err(TrySendError::Full(FolderSyncEvent::Scan(_, _))) => {}
    }
}

impl crate::reader::Reader {
    pub(crate) fn retry_deferred_annotation_saves(&mut self) {
        if let Some(job) = self.unqueueable_annotation_save.take()
            && self.folder_sync.save(job.clone()).is_err()
        {
            self.unqueueable_annotation_save = Some(job);
            return;
        }
        while let Some(job) = self.deferred_annotation_saves.pop_front() {
            if let Err(reason) = self.folder_sync.save(job.clone()) {
                self.deferred_annotation_saves.push_front(job);
                if !self.deferred_status_shown {
                    self.status = Some(format!("Annotations waiting to save: {reason}"));
                    self.deferred_status_shown = true;
                    self.redraw();
                }
                return;
            }
        }
        self.deferred_status_shown = false;
    }

    pub(crate) fn defer_annotation_save(&mut self, job: PendingSave) -> &'static str {
        let coalesced = self
            .deferred_annotation_saves
            .iter()
            .any(|pending| pending.sidecar == job.sidecar);
        if retain_deferred(
            &mut self.deferred_annotation_saves,
            &mut self.unqueueable_annotation_save,
            job,
        ) {
            if coalesced {
                "annotation worker queue full; latest snapshot retained for retry"
            } else if self.unqueueable_annotation_save.is_some() {
                "retry queue full; current document is held until storage catches up"
            } else {
                "annotation worker queue full; snapshot retained for retry"
            }
        } else {
            "retry queue full; latest in-memory document cannot switch until storage catches up"
        }
    }

    pub(crate) fn poll_folder_sync(&mut self) {
        let mut library_changed = false;
        let mut status_changed = false;
        self.retry_deferred_annotation_saves();
        while let Some(event) = self.folder_sync.try_event() {
            match event {
                FolderSyncEvent::Scan(documents, annotations) => {
                    let old = std::mem::replace(&mut self.synced_documents, documents);
                    let conflict = self
                        .synced_documents
                        .iter()
                        .find(|new| {
                            old.iter()
                                .chain(self.synced_documents.iter())
                                .any(|previous| {
                                    previous.relative_path == new.relative_path
                                        && previous.source_hash != new.source_hash
                                })
                        })
                        .map(|document| document.relative_path.display().to_string());
                    self.synced_annotations = annotations.into_iter().collect();
                    library_changed = old != self.synced_documents;
                    if self.annotation_backup_restore_allowed
                        && self.annotations.is_empty()
                        && let Some(path) = self.doc_path.as_deref().map(Path::new)
                        && let Some(document) = self
                            .synced_documents
                            .iter()
                            .find(|document| document.local_path == path)
                        && let Some(restored) = self.synced_annotations.get(&document.id)
                    {
                        self.annotations = restored.clone();
                        self.annotation_backup_restore_allowed = false;
                        if self.annot_sidecar.is_some() {
                            self.save_annotations();
                        }
                    }
                    if let Some(path) = conflict {
                        self.status = Some(format!(
                            "New source version: {}. Previous notes remain with the earlier copy.",
                            path
                        ));
                        status_changed = true;
                    } else if library_changed {
                        self.status = Some(format!(
                            "Folder sync: {} PDFs available",
                            self.synced_documents.len()
                        ));
                        status_changed = true;
                    }
                }
                FolderSyncEvent::Status(status) => {
                    let show_feedback = status.starts_with("Annotations published:")
                        || status.contains("failed")
                        || status.contains("unavailable")
                        || status.starts_with("Cannot ");
                    self.status = Some(status);
                    status_changed = true;
                    if show_feedback && let Some(message) = self.status.clone() {
                        self.show_toast(&message);
                    }
                }
            }
        }
        if library_changed {
            self.refresh_curated_library_data();
        }
        if library_changed || status_changed {
            self.redraw();
        }
    }

    pub(crate) fn flush_annotation_publication(&mut self) {
        self.retry_deferred_annotation_saves();
        self.folder_sync.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActorState, FolderSyncConfig, FolderSyncWorker, MAX_DEFERRED_DOCUMENTS,
        MAX_PENDING_DOCUMENTS, PendingSave,
    };
    use pdf_core::AnnotationSet;
    use pdf_core::annotations::{Annotation, TextNote};
    use pdf_core::store::AnnotationStore;
    use std::collections::VecDeque;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::{Arc, Condvar, Mutex, mpsc};
    use std::time::{Duration, Instant};

    #[test]
    fn folder_sync_is_disabled_when_configuration_is_missing_or_incomplete() {
        assert!(!FolderSyncConfig::default().is_active());
        assert!(
            !FolderSyncConfig {
                enabled: true,
                incoming_root: Some(PathBuf::from("/incoming")),
                outgoing_root: None,
            }
            .is_active()
        );
    }

    #[test]
    fn active_configuration_requires_both_roots() {
        let config = FolderSyncConfig {
            enabled: true,
            incoming_root: Some(PathBuf::from("/incoming")),
            outgoing_root: Some(PathBuf::from("/outgoing")),
        };
        assert!(config.is_active());
    }

    #[test]
    fn relative_roots_do_not_activate_sync() {
        let config = FolderSyncConfig {
            enabled: true,
            incoming_root: Some(PathBuf::from("Master")),
            outgoing_root: Some(PathBuf::from("Master")),
        };
        assert!(!config.is_active());
    }

    #[test]
    fn config_uses_the_private_folder_sync_filename_and_defaults_disabled() {
        let base =
            std::env::temp_dir().join(format!("pdflector-sync-config-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        fs::write(
            base.join("folder_sync.json"),
            br#"{"enabled":true,"incoming_root":"/in","outgoing_root":"/out"}"#,
        )
        .unwrap();
        let loaded = FolderSyncConfig::load(&base);
        assert!(loaded.is_active());
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn pending_queue_coalesces_same_sidecar_and_enforces_document_bound() {
        let shared = Arc::new((Mutex::new(ActorState::default()), Condvar::new()));
        let worker = FolderSyncWorker {
            shared: Arc::clone(&shared),
            events: mpsc::sync_channel(1).1,
            latest_status: Arc::new(Mutex::new(None)),
            thread: None,
            available: true,
        };
        let make_job = |name: &str| PendingSave {
            pdf_path: PathBuf::from(format!("/{name}.pdf")),
            sidecar: PathBuf::from(format!("/{name}.db")),
            annotations: AnnotationSet::new(),
            publish: false,
            persisted: false,
        };
        worker.save(make_job("same")).unwrap();
        worker.save(make_job("same")).unwrap();
        assert_eq!(shared.0.lock().unwrap().pending.len(), 1);
        for index in 0..MAX_PENDING_DOCUMENTS - 1 {
            worker.save(make_job(&format!("doc-{index}"))).unwrap();
        }
        assert_eq!(
            shared.0.lock().unwrap().pending.len(),
            MAX_PENDING_DOCUMENTS
        );
        assert!(worker.save(make_job("overflow")).is_err());
    }

    #[test]
    fn annotation_worker_serializes_snapshots_to_sqlite_before_export_delay() {
        let base =
            std::env::temp_dir().join(format!("pdflector-sync-worker-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let worker = FolderSyncWorker::start(base.clone(), FolderSyncConfig::default());
        let pdf_path = base.join("book.pdf");
        let sidecar = base.join("book.db");
        let mut final_set = AnnotationSet::new();
        final_set.add(
            0,
            Annotation::TextNote(TextNote {
                anchor: (2.0, 3.0),
                text: "latest".into(),
            }),
        );
        let start = Instant::now();
        for text in ["older", "latest"] {
            let mut annotations = AnnotationSet::new();
            annotations.add(
                0,
                Annotation::TextNote(TextNote {
                    anchor: (2.0, 3.0),
                    text: text.into(),
                }),
            );
            worker
                .save(PendingSave {
                    pdf_path: pdf_path.clone(),
                    sidecar: sidecar.clone(),
                    annotations,
                    publish: false,
                    persisted: false,
                })
                .unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut saved = false;
        while Instant::now() < deadline {
            if let Some(super::FolderSyncEvent::Status(status)) = worker.try_event()
                && status == "Annotations saved"
            {
                let store = AnnotationStore::open(&sidecar).unwrap();
                if store.load().unwrap() == final_set {
                    saved = true;
                    assert!(
                        start.elapsed() < Duration::from_millis(600),
                        "SQLite save inherited the export debounce"
                    );
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saved, "worker did not persist the latest snapshot");
        drop(worker);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn annotation_worker_replays_a_pending_journal_after_restart() {
        let base =
            std::env::temp_dir().join(format!("pdflector-sync-recovery-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        let sidecar = base.join("book.db");
        let mut annotations = AnnotationSet::new();
        annotations.add(
            0,
            Annotation::TextNote(TextNote {
                anchor: (1.0, 1.0),
                text: "recovered".into(),
            }),
        );
        let pending = PendingSave {
            pdf_path: base.join("book.pdf"),
            sidecar: sidecar.clone(),
            annotations: annotations.clone(),
            publish: false,
            persisted: false,
        };
        let journal = base.join("folder_sync/pending.json");
        super::write_json_atomic(&journal, &vec![pending]).unwrap();

        let worker = FolderSyncWorker::start(base.clone(), FolderSyncConfig::default());
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut recovered = false;
        while Instant::now() < deadline {
            let _ = worker.try_event();
            if sidecar.is_file()
                && AnnotationStore::open(&sidecar)
                    .and_then(|store| store.load())
                    .is_ok_and(|loaded| loaded == annotations)
            {
                recovered = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(recovered, "worker did not replay its pending journal");
        drop(worker);
        let _ = fs::remove_dir_all(base);
    }

    #[test]
    fn failed_old_save_does_not_replace_a_newer_queued_snapshot() {
        let sidecar = PathBuf::from("/book.db");
        let make_job = |text: &str| {
            let mut annotations = AnnotationSet::new();
            annotations.add(
                0,
                Annotation::TextNote(TextNote {
                    anchor: (0.0, 0.0),
                    text: text.into(),
                }),
            );
            PendingSave {
                pdf_path: PathBuf::from("/book.pdf"),
                sidecar: sidecar.clone(),
                annotations,
                publish: false,
                persisted: false,
            }
        };
        let newest = make_job("newest");
        let mut state = ActorState {
            pending: VecDeque::from([newest.clone()]),
            ..ActorState::default()
        };
        super::requeue_failed_job(&mut state, make_job("stale"));
        assert_eq!(state.pending.len(), 1);
        assert_eq!(
            state.pending.front().unwrap().annotations,
            newest.annotations
        );
    }

    #[test]
    fn deferred_overflow_keeps_a_separate_active_snapshot() {
        let make_job = |index: usize| PendingSave {
            pdf_path: PathBuf::from(format!("/doc-{index}.pdf")),
            sidecar: PathBuf::from(format!("/doc-{index}.db")),
            annotations: AnnotationSet::new(),
            publish: false,
            persisted: false,
        };
        let mut queue = VecDeque::new();
        let mut overflow = None;
        for index in 0..MAX_DEFERRED_DOCUMENTS + 1 {
            assert!(super::retain_deferred(
                &mut queue,
                &mut overflow,
                make_job(index)
            ));
        }
        assert_eq!(queue.len(), MAX_DEFERRED_DOCUMENTS);
        assert_eq!(
            overflow.as_ref().unwrap().pdf_path,
            PathBuf::from("/doc-64.pdf")
        );
        assert!(!super::retain_deferred(
            &mut queue,
            &mut overflow,
            make_job(65)
        ));
        assert_eq!(
            overflow.as_ref().unwrap().pdf_path,
            PathBuf::from("/doc-64.pdf")
        );
    }

    #[test]
    fn local_save_failure_wakes_for_retry_without_waiting_for_a_folder_scan() {
        let mut state = ActorState::default();
        state.pending.push_back(PendingSave {
            pdf_path: PathBuf::from("/book.pdf"),
            sidecar: PathBuf::from("/book.db"),
            annotations: AnnotationSet::new(),
            publish: false,
            persisted: false,
        });
        let retry_at = Instant::now() + Duration::from_secs(5);
        let delay = super::next_wake_delay(
            &state,
            false,
            Some(retry_at),
            Instant::now() + Duration::from_secs(30),
            false,
        );
        assert!(delay <= Duration::from_secs(5));
        assert!(delay > Duration::from_secs(4));
    }

    #[test]
    fn status_mailbox_is_delivered_after_older_queued_statuses() {
        let shared = Arc::new((Mutex::new(ActorState::default()), Condvar::new()));
        let (sender, events) = mpsc::sync_channel(1);
        let latest_status = Arc::new(Mutex::new(None));
        assert!(
            sender
                .send(super::FolderSyncEvent::Status("older".into()))
                .is_ok()
        );
        super::emit(
            &sender,
            &latest_status,
            super::FolderSyncEvent::Status("newer error".into()),
        );
        let worker = FolderSyncWorker {
            shared,
            events,
            latest_status,
            thread: None,
            available: true,
        };
        assert!(
            matches!(worker.try_event(), Some(super::FolderSyncEvent::Status(text)) if text == "older")
        );
        assert!(
            matches!(worker.try_event(), Some(super::FolderSyncEvent::Status(text)) if text == "newer error")
        );
    }
}
