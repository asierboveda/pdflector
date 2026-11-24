use std::fs::{self, FileTimes, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use pdf_core::folder_sync::FolderLibrary;
use pdf_core::{Annotation, AnnotationSet, TextNote};

struct TestDirs(PathBuf);

impl TestDirs {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("pdflector-folder-sync-{nonce}"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }
}

impl Drop for TestDirs {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn library(dirs: &TestDirs) -> FolderLibrary {
    FolderLibrary::open(
        dirs.path("internal"),
        dirs.path("incoming"),
        dirs.path("outgoing"),
    )
    .unwrap()
}

#[test]
fn folder_library_can_be_owned_by_a_background_worker() {
    fn assert_send<T: Send>() {}
    assert_send::<FolderLibrary>();
}

fn pdf_bytes() -> Vec<u8> {
    include_bytes!("assets/simple.pdf").to_vec()
}

fn note_set(text: &str) -> AnnotationSet {
    let mut set = AnnotationSet::new();
    set.add(
        0,
        Annotation::TextNote(TextNote {
            text: text.to_owned(),
            anchor: (12.0, 18.0),
        }),
    );
    set
}

#[test]
fn scan_preserves_relative_paths_and_distinguishes_duplicate_names() {
    let dirs = TestDirs::new();
    for folder in ["incoming/Lectures", "incoming/Books"] {
        fs::create_dir_all(dirs.path(folder)).unwrap();
        fs::write(dirs.path(folder).join("same.pdf"), pdf_bytes()).unwrap();
    }

    let mut library = library(&dirs);
    let docs = library.scan().unwrap();
    assert_eq!(docs.len(), 2);
    assert!(
        docs.iter()
            .any(|doc| doc.relative_path == Path::new("Lectures/same.pdf"))
    );
    assert!(
        docs.iter()
            .any(|doc| doc.relative_path == Path::new("Books/same.pdf"))
    );
    assert_ne!(docs[0].id, docs[1].id);
    assert!(
        docs.iter()
            .all(|doc| doc.page_count > 0 && doc.local_path.exists())
    );
    assert_ne!(docs[0].local_path, docs[1].local_path);
}

#[test]
fn scan_imports_a_bounded_batch_and_reports_remaining_work() {
    let dirs = TestDirs::new();
    for index in 0..5 {
        let path = dirs.path(&format!("incoming/course-{index}/book.pdf"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, pdf_bytes()).unwrap();
    }
    let mut library = library(&dirs);
    assert_eq!(library.scan().unwrap().len(), 4);
    assert!(library.has_more_work());
    assert_eq!(library.scan().unwrap().len(), 5);
    assert!(!library.has_more_work());
}

#[test]
fn scan_excludes_pdflector_and_invalid_or_partial_pdfs() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/course/_PDFLector/Apuntes")).unwrap();
    fs::create_dir_all(dirs.path("incoming/course")).unwrap();
    fs::write(dirs.path("incoming/course/valid.pdf"), pdf_bytes()).unwrap();
    fs::write(
        dirs.path("incoming/course/_PDFLector/Apuntes/annotated.pdf"),
        pdf_bytes(),
    )
    .unwrap();
    fs::write(
        dirs.path("incoming/course/partial.pdf"),
        b"%PDF-1.7 truncated",
    )
    .unwrap();
    fs::write(dirs.path("incoming/course/not-pdf.txt"), b"hello").unwrap();

    let docs = library(&dirs).scan().unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].relative_path, Path::new("course/valid.pdf"));
}

#[cfg(unix)]
#[test]
fn scan_does_not_follow_symlinked_pdf_files_or_directories() {
    use std::os::unix::fs::symlink;

    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/course")).unwrap();
    fs::create_dir_all(dirs.path("outside")).unwrap();
    fs::write(dirs.path("outside/external.pdf"), pdf_bytes()).unwrap();
    symlink(
        dirs.path("outside/external.pdf"),
        dirs.path("incoming/course/file.pdf"),
    )
    .unwrap();
    symlink(
        dirs.path("outside"),
        dirs.path("incoming/course/linked-folder"),
    )
    .unwrap();

    assert!(library(&dirs).scan().unwrap().is_empty());
}

#[test]
fn source_replacement_creates_immutable_version_and_source_deletion_keeps_import() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/course")).unwrap();
    let source = dirs.path("incoming/course/book.pdf");
    fs::write(&source, pdf_bytes()).unwrap();
    let mut library = library(&dirs);
    let first = library.scan().unwrap().remove(0);
    let first_bytes = fs::read(&first.local_path).unwrap();

    let mut replaced = pdf_bytes();
    replaced.extend_from_slice(b"\n% newer source revision\n");
    fs::write(&source, replaced).unwrap();
    let docs = library.scan().unwrap();
    assert_eq!(docs.len(), 2);
    assert_ne!(docs[0].id, docs[1].id);
    assert_eq!(fs::read(&first.local_path).unwrap(), first_bytes);
    fs::remove_file(source).unwrap();
    assert_eq!(library.scan().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn same_size_rewrite_with_restored_mtime_is_detected() {
    use std::os::unix::fs::MetadataExt;

    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/course")).unwrap();
    let source = dirs.path("incoming/course/book.pdf");
    fs::write(&source, pdf_bytes()).unwrap();
    let mut library = library(&dirs);
    let old_doc = library.scan().unwrap().remove(0);
    let before = fs::metadata(&source).unwrap();
    let modified = before.modified().unwrap();
    let old_size = before.len();
    let old_ctime = (before.ctime(), before.ctime_nsec());

    let mut file = OpenOptions::new().write(true).open(&source).unwrap();
    file.seek(SeekFrom::Start(7)).unwrap();
    file.write_all(b"4").unwrap();
    file.sync_all().unwrap();
    file.set_times(FileTimes::new().set_modified(modified))
        .unwrap();

    let after = fs::metadata(&source).unwrap();
    assert_eq!(after.len(), old_size);
    assert_eq!(after.modified().unwrap(), modified);
    assert_ne!((after.ctime(), after.ctime_nsec()), old_ctime);
    let docs = library.scan().unwrap();
    assert_eq!(docs.len(), 2);
    assert!(docs.iter().any(|doc| doc.id != old_doc.id));
}

#[test]
fn changed_source_does_not_restore_annotations_from_its_previous_version() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/course")).unwrap();
    let source = dirs.path("incoming/course/book.pdf");
    fs::write(&source, pdf_bytes()).unwrap();
    let mut library = library(&dirs);
    let old_doc = library.scan().unwrap().remove(0);
    library
        .publish(&old_doc.local_path, &note_set("old version"))
        .unwrap();

    let mut replacement = pdf_bytes();
    replacement.extend_from_slice(b"\n% replacement\n");
    fs::write(source, replacement).unwrap();
    let docs = library.scan().unwrap();
    let new_doc = docs.iter().find(|doc| doc.id != old_doc.id).unwrap();
    assert!(library.annotations(&new_doc.id).is_none());
    library
        .publish(&new_doc.local_path, &note_set("new version"))
        .unwrap();
    assert!(
        dirs.path("outgoing/_PDFLector/Apuntes/course")
            .read_dir()
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("book [PDFLector-"))
    );
    let mut fresh = FolderLibrary::open(
        dirs.path("fresh-internal"),
        dirs.path("incoming"),
        dirs.path("outgoing"),
    )
    .unwrap();
    let fresh_docs = fresh.scan().unwrap();
    let restored = fresh_docs
        .iter()
        .find(|doc| doc.source_hash == new_doc.source_hash)
        .unwrap();
    assert_eq!(
        fresh.annotations(&restored.id),
        Some(&note_set("new version"))
    );
}

#[test]
fn publication_never_overwrites_an_existing_unowned_file() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/Lectures")).unwrap();
    fs::create_dir_all(dirs.path("outgoing/_PDFLector/Apuntes/Lectures")).unwrap();
    fs::create_dir_all(dirs.path("outgoing/_PDFLector/Respaldo/Lectures")).unwrap();
    fs::write(dirs.path("incoming/Lectures/topic.pdf"), pdf_bytes()).unwrap();
    let existing = dirs.path("outgoing/_PDFLector/Apuntes/Lectures/topic.pdf");
    fs::write(&existing, b"existing user file").unwrap();
    let mut opened = library(&dirs);
    let doc = opened.scan().unwrap().remove(0);
    opened
        .publish(&doc.local_path, &note_set("published elsewhere"))
        .unwrap();
    assert_eq!(fs::read(existing).unwrap(), b"existing user file");
    assert!(
        dirs.path("outgoing/_PDFLector/Apuntes/Lectures")
            .read_dir()
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("topic [PDFLector-"))
    );
}

#[cfg(unix)]
#[test]
fn publication_rejects_symlinked_output_directories() {
    use std::os::unix::fs::symlink;

    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/Lectures")).unwrap();
    fs::create_dir_all(dirs.path("outside")).unwrap();
    fs::write(dirs.path("incoming/Lectures/topic.pdf"), pdf_bytes()).unwrap();
    fs::create_dir_all(dirs.path("outgoing/_PDFLector")).unwrap();
    symlink(
        dirs.path("outside"),
        dirs.path("outgoing/_PDFLector/Apuntes"),
    )
    .unwrap();

    let mut library = library(&dirs);
    let doc = library.scan().unwrap().remove(0);
    assert!(
        library
            .publish(&doc.local_path, &note_set("blocked"))
            .is_err()
    );
    assert!(fs::read_dir(dirs.path("outside")).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn library_roots_canonicalize_symlinked_ancestors_but_reject_descendants() {
    use std::os::unix::fs::symlink;

    let dirs = TestDirs::new();
    let real = dirs.path("real");
    fs::create_dir_all(real.join("incoming/Books")).unwrap();
    fs::write(real.join("incoming/Books/book.pdf"), pdf_bytes()).unwrap();
    symlink(&real, dirs.path("alias")).unwrap();

    let mut library = FolderLibrary::open(
        dirs.path("alias/internal"),
        dirs.path("alias/incoming"),
        dirs.path("alias/outgoing"),
    )
    .unwrap();
    let documents = library.scan().unwrap();
    assert_eq!(documents.len(), 1);
    assert!(real.join("internal/folder-library.json").is_file());

    fs::create_dir_all(real.join("outside")).unwrap();
    fs::create_dir_all(real.join("outgoing/_PDFLector")).unwrap();
    symlink(
        real.join("outside"),
        real.join("outgoing/_PDFLector/Apuntes"),
    )
    .unwrap();
    assert!(
        library
            .publish(&documents[0].local_path, &note_set("blocked"))
            .is_err()
    );
}

#[test]
fn interrupted_publication_is_retried_when_library_reopens() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/Books")).unwrap();
    fs::write(dirs.path("incoming/Books/book.pdf"), pdf_bytes()).unwrap();
    let mut opened = library(&dirs);
    let doc = opened.scan().unwrap().remove(0);
    let set = note_set("recover after restart");
    let dirty = dirs.path(&format!("internal/dirty/{}.json", doc.id));
    fs::create_dir_all(dirty.parent().unwrap()).unwrap();
    let backup = serde_json::json!({
        "backup": {
            "schema": 1,
            "source_hash": doc.source_hash,
            "source_path": doc.relative_path,
            "annotations": set,
        },
        "publication": {
            "pdf_relative": doc.relative_path,
            "backup_relative": "Books/book.pdf.json",
        },
    });
    fs::write(&dirty, serde_json::to_vec(&backup).unwrap()).unwrap();
    drop(opened);

    let reopened = library(&dirs);
    assert!(!dirty.exists());
    assert!(
        dirs.path("outgoing/_PDFLector/Apuntes/Books/book.pdf")
            .exists()
    );
    assert_eq!(reopened.annotations(&doc.id), Some(&set));
}

#[test]
fn publish_is_repeatable_and_writes_portable_pdf_and_annotation_backup() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/Lectures")).unwrap();
    fs::write(dirs.path("incoming/Lectures/topic.pdf"), pdf_bytes()).unwrap();
    let mut library = library(&dirs);
    let doc = library.scan().unwrap().remove(0);
    let set = note_set("remember this");
    library.publish(&doc.local_path, &set).unwrap();
    let pdf_out = dirs.path("outgoing/_PDFLector/Apuntes/Lectures/topic.pdf");
    let backup_out = dirs.path("outgoing/_PDFLector/Respaldo/Lectures/topic.pdf.json");
    assert!(pdf_out.exists());
    let backup: serde_json::Value =
        serde_json::from_slice(&fs::read(&backup_out).unwrap()).unwrap();
    assert_eq!(backup["schema"], 1);
    assert_eq!(backup["source_hash"], doc.source_hash);
    assert_eq!(
        backup["annotations"]["by_page"]["0"][0]["kind"]["TextNote"]["text"],
        "remember this"
    );

    library.publish(&doc.local_path, &set).unwrap();
    let exported = mupdf::pdf::PdfDocument::open(pdf_out.to_str().unwrap()).unwrap();
    let page = exported.load_pdf_page(0).unwrap();
    assert_eq!(
        page.annotations().count(),
        1,
        "re-export must not duplicate embedded annotations"
    );
    assert_eq!(
        fs::read_dir(dirs.path("outgoing/_PDFLector/Apuntes/Lectures"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn backup_annotations_are_recovered_after_reopen_and_matching_scan() {
    let dirs = TestDirs::new();
    fs::create_dir_all(dirs.path("incoming/Books")).unwrap();
    fs::write(dirs.path("incoming/Books/book.pdf"), pdf_bytes()).unwrap();
    let set = note_set("portable");
    {
        let mut library = library(&dirs);
        let doc = library.scan().unwrap().remove(0);
        library.publish(&doc.local_path, &set).unwrap();
    }

    let mut reopened = library(&dirs);
    let doc = reopened.scan().unwrap().remove(0);
    assert_eq!(reopened.annotations(&doc.id).unwrap(), &set);
}
