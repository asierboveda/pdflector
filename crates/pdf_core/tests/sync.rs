//! Integration tests for the sync module (Fase 4, PLAN §3.5): the
//! sync-friendly layout helpers and live change detection with `notify`.
//! No network involved — Syncthing is external; these tests exercise only
//! the local trigger. The watcher tests use real filesystem events, so they
//! allow a generous timeout to avoid flakiness on slow CI machines.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use pdf_core::store::sidecar_path;
use pdf_core::sync::{AnnotationWatcher, annotations_dir, library_index_path, watch_annotations};

/// Unique per-test temp dir, removed on drop (also on panic).
struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("pdflector-sync-test-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn annotations_dir_follows_plan_convention() {
    // Coherent with store::sidecar_path by construction: the sidecar lives
    // inside this directory.
    let cases = [
        (Path::new("/lib/doc.pdf"), Path::new("/lib/annotations")),
        (
            Path::new("libs/sub/doc.pdf"),
            Path::new("libs/sub/annotations"),
        ),
        (Path::new("/lib/a.b.pdf"), Path::new("/lib/annotations")),
        (Path::new("doc.pdf"), Path::new("annotations")),
    ];
    for (pdf, want) in cases {
        assert_eq!(annotations_dir(pdf), want, "for {pdf:?}");
        // The sidecar is always inside the annotations dir.
        assert!(
            sidecar_path(pdf).starts_with(annotations_dir(pdf)),
            "for {pdf:?}"
        );
    }
}

#[test]
fn library_index_lives_at_library_root() {
    assert_eq!(
        library_index_path(Path::new("/lib")),
        Path::new("/lib/library.db")
    );
    assert_eq!(
        library_index_path(Path::new("lib")),
        Path::new("lib/library.db")
    );
    // Any path is accepted verbatim; no canonicalisation.
    assert_eq!(
        library_index_path(Path::new("/data/docs/")),
        Path::new("/data/docs/library.db")
    );
}

/// Watches `pdf` and returns a receiver that gets a message per `on_change`
/// call, plus the watcher (kept alive for the duration of the test).
fn watch(pdf: &Path) -> (AnnotationWatcher, mpsc::Receiver<()>) {
    let (tx, rx) = mpsc::channel();
    let watcher = watch_annotations(pdf, move || {
        let _ = tx.send(());
    })
    .expect("watch registers");
    (watcher, rx)
}

/// Waits up to `limit` for a change notification. The debounce thread
/// already coalesces a burst into a single `on_change`, so the first
/// message is enough. Returns false when nothing arrived in time.
fn wait_for_change(rx: &mpsc::Receiver<()>, limit: Duration) -> bool {
    matches!(rx.recv_timeout(limit), Ok(()))
}

#[test]
fn watch_fires_on_sidecar_change() {
    let tmp = TempDir::new("sidecar-change");
    let pdf = tmp.path().join("libro.pdf");
    std::fs::write(&pdf, b"pdf-stub").expect("write pdf");
    // Pre-existing annotation set, as after a first save.
    let db = sidecar_path(&pdf);
    std::fs::create_dir_all(db.parent().expect("annotations dir")).expect("create annotations dir");
    std::fs::write(&db, b"old").expect("write sidecar");

    let (_watcher, rx) = watch(&pdf);

    std::fs::write(&db, b"new").expect("rewrite sidecar");
    assert!(
        wait_for_change(&rx, Duration::from_secs(10)),
        "sidecar rewrite must trigger on_change"
    );
}

#[test]
fn watch_tolerates_missing_annotations_dir() {
    // A never-annotated PDF: no `annotations/` yet. The watcher must still
    // register (falling back to the PDF's parent) and fire when Syncthing
    // creates the directory with the sidecar inside.
    let tmp = TempDir::new("missing-ann");
    let pdf = tmp.path().join("libro.pdf");
    std::fs::write(&pdf, b"pdf-stub").expect("write pdf");

    let (_watcher, rx) = watch(&pdf);

    let db = sidecar_path(&pdf);
    std::fs::create_dir_all(db.parent().expect("annotations dir")).expect("create annotations dir");
    std::fs::write(&db, b"sync").expect("write sidecar");
    assert!(
        wait_for_change(&rx, Duration::from_secs(10)),
        "appearance of annotations/ must trigger on_change"
    );
}

#[test]
fn watch_rejects_missing_pdf() {
    // A PDF whose parent directory does not exist either: there is nothing
    // watchable at all, so registration must fail.
    let missing_dir = std::env::temp_dir().join(format!("pdflector-no-dir-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing_dir);
    let pdf = missing_dir.join("no-such.pdf");
    assert!(watch_annotations(&pdf, || {}).is_err());
}
