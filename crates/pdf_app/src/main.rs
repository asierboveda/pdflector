//! pdf_app — egui desktop prototype (docs/PLAN.md: final Android UI decided in
//! Phase 6; this app never holds logic, it only asks pdf_core and paints).
//!
//! Fase 1 "fluent reading": continuous virtualized scroll of the whole
//! document. The UI thread never renders (AGENTS.md §4.6): all caching and
//! rendering live in `pdf_core::prefetch::Prefetcher`'s worker thread; this
//! app only translates the visible window into a `Viewport`, polls
//! `Prefetcher::get_page` receivers with `try_recv` and uploads landing
//! bitmaps to GPU textures.
//!
//! Fase 2 additions, both pure presentation concerns (no pdf_core change): a
//! dark-mode toggle that inverts pages at texture upload (the cache always
//! keeps the normal bitmaps) and persists in eframe's storage; and a debug
//! overlay showing frame-time p95, RSS and the prefetcher's cache counters.
//!
//! Fase 3 addition: a vector annotation layer (AGENTS.md §4.3). A draw-mode
//! toggle turns a primary-button drag over a page into a freehand `Stroke`
//! stored in `pdf_core::AnnotationSet` (page coordinates) and painted every
//! frame on top of the page textures with the egui painter. The
//! cursor→page transform is the exact inverse of the page placement (see
//! `page_rect`/`screen_to_page`).
//!
//! Fase 3-4 closure: persistence, export and sync are wired into the same
//! layer. The set is loaded from the SQLite sidecar at open and saved on
//! every mutation (`AnnotationStore`, kept alive in `App::store`); the
//! toolbar exports it to Markdown and to an annotated PDF copy (background
//! thread, `start_export`); and a `notify` watcher (Fase 4) hot-reloads the
//! set when the sidecar changes on disk — Syncthing is the one that copies,
//! this is only the local trigger (`App::sync_rx` / `reload_annotations`).
//!
//! Fase 3.5 additions, still pure presentation (no pdf_core change): two more
//! tools beside Draw (`ToolMode`) and an annotations panel. Highlight turns a
//! drag into a per-line `Highlight` computed from the page text on a
//! background thread (`highlight_worker`, the MuPDF context is per-thread TLS
//! so the prefetcher's document never leaves its worker — same pattern as
//! `chat_worker`); Note turns a click into a `TextNote` anchored at the
//! click, typed in a small floating input. Both persist through the existing
//! sidecar store (`save_annotations`). The toolbar "Annotations" toggle opens
//! a side panel listing every annotation (page + type + summary); clicking
//! one jumps and centers it via `ui.scroll_to_rect`. Finally, the last
//! `RECENTS_MAX` opened PDFs are kept in eframe's storage (`recent_pdfs`,
//! RON through `eframe::get_value`/`set_value` — the persistence feature
//! already backs dark mode) and offered in the Open menu and the empty state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::prefetch::Prefetcher;
use pdf_core::store::{AnnotationStore, sidecar_path};
use pdf_core::sync::{AnnotationWatcher, watch_annotations};
use pdf_core::{
    Annotation, AnnotationSet, Bitmap, Color, Document, FrameTimer, Highlight, Rect, RenderEngine,
    Stroke, TextNote, TextSpan, Viewport, invert_bitmap, read_rss_kb, scale_level_for_zoom,
};

/// Zoom range and per-click step (Fase 1 B3). The continuous `zoom` factor is
/// clamped to this range; `+`/`−` step by `ZOOM_STEP`, ctrl+wheel and trackpad
/// pinch follow egui's `zoom_delta` (see `App::update`).
const ZOOM_MIN: f32 = 0.25;
const ZOOM_MAX: f32 = 8.0;
const ZOOM_STEP: f32 = 1.1;

/// Byte budget for the render cache (owned by the Prefetcher worker).
///
/// 32 MB keeps total RSS well under the < 150 MB target (AGENTS.md §8): one
/// A4 page at screen resolution on a 2× display ≈ 8 MB, so ≈ 4 pages stay
/// resident — the visible window plus a couple of prefetched neighbours.
/// Bigger budgets would eat the RAM budget; smaller ones would thrash while
/// scrolling (evict a page, re-render it a moment later). A single page at a
/// very high zoom level can exceed the budget by itself; that is documented
/// best-effort behaviour of `pdf_core::cache::RenderCache`.
const BYTE_BUDGET: usize = 32 * 1024 * 1024;

/// Upper bound for the prefetch radius (pages around the visible window)
/// submitted to the actor. The actual radius is shrunk automatically so the
/// requested window fits the byte budget at the current ladder level (see
/// `App::request_radius`): when a request exceeds the budget, pdf_core's LRU
/// evicts the visible pages (rendered first = least recently used) and the
/// placeholders never fill in.
const PREFETCH_RADIUS_MAX: usize = 2;

/// Vertical gap between pages, logical pixels.
const PAGE_GAP: f32 = 8.0;

/// Default width of a freehand stroke, in PDF points (Fase 3 draw mode). The
/// on-screen width is `pt × zoom`, so the pen looks the same at any zoom.
const STROKE_WIDTH_PT: f32 = 1.5;

/// Default stroke colour (RGBA): orange-red, semi-opaque — readable over
/// both light and dark pages.
const STROKE_COLOR: Color = Color {
    r: 230,
    g: 60,
    b: 30,
    a: 230,
};

/// Minimum distance between two captured points, in screen pixels. Without a
/// threshold a 60 fps drag stores a point per frame at any speed, bloating
/// the polyline with sub-pixel noise; 2 px keeps scribbles smooth and small.
const MIN_POINT_DIST_PX: f32 = 2.0;

/// Texture keep-alive margin: pages this far outside the visible window keep
/// their GPU texture (and get one requested if missing). Covers the prefetch
/// radius, so textures survive small scroll jitter without re-uploading.
const TEXTURE_MARGIN: usize = 1;

/// Minimum interval between `get_page` queries for the same page: a page that
/// is not resident yet is retried at this rate instead of every frame, so a
/// worker busy rendering is not flooded with snapshot commands.
const GET_PAGE_RETRY: Duration = Duration::from_millis(33);

/// Storage key for the dark-mode preference in eframe's storage (a RON file
/// in the XDG data dir when the `persistence` feature is on).
const KEY_DARK_MODE: &str = "dark_mode";

/// Frames separated by more than this are treated as an idle gap, not a slow
/// frame: egui stops calling `update` when nothing repaints, so the elapsed
/// time after an idle pause would otherwise poison the debug overlay's p95
/// with seconds-long "frames". 250 ms is far above the worst real frame (a
/// zoom level-change re-render) but far below an idle pause.
const ACTIVE_FRAME_MAX: Duration = Duration::from_millis(250);

/// Minimum interval between `request()` submissions. During fast scrolling the
/// viewport changes every frame; each submission replaces the actor's whole
/// wishlist, which the worker drains serially (FIFO, no cancellation — B2
/// minimal). Throttling bounds the backlog so the current viewport's pages
/// render promptly once the scroll pauses, instead of queuing behind dozens of
/// stale windows. Slow scrolling (a page per second) is never throttled.
const REQUEST_INTERVAL: Duration = Duration::from_millis(100);

// Fase 3.5 — highlight / text note / annotations panel / recents.

/// Default highlight colour (RGBA): classic marker yellow, semi-transparent
/// so the underlying text stays readable. Stored in the annotation (so a
/// future colour picker only changes this constant / the add site).
const HIGHLIGHT_COLOR: Color = Color {
    r: 255,
    g: 230,
    b: 0,
    a: 110,
};

/// Screen size of the text-note marker (a small filled square centred on the
/// anchor), constant across zooms — like the cursor, not the page content.
const NOTE_MARKER_PX: f32 = 12.0;

/// Extra hit area around the marker for the hover tooltip (a 12 px square is
/// too small to aim at comfortably).
const NOTE_HOVER_PAD: f32 = 6.0;

/// Marker fill (amber, readable over light and dark pages).
const NOTE_MARKER_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 180, 60);

/// Marker border, a thin darker outline so the marker stands out on any page.
const NOTE_MARKER_BORDER: egui::Color32 = egui::Color32::from_rgb(120, 80, 20);

/// Text used for a note committed with an empty input.
const NOTE_DEFAULT_TEXT: &str = "Nota";

/// How many recently opened PDFs are kept (most recent first).
const RECENTS_MAX: usize = 5;

/// Storage key for the recents list in eframe's storage (a RON value, see
/// `App::new` / `save`).
const KEY_RECENTS: &str = "recent_pdfs";

// Fase 5 — AI chat panel (docs/PLAN.md §5). The chat is best-effort and
// optional: it only needs Ollama reachable on the local network; when it is
// not, the app keeps working and the panel shows a clear error.

/// Default Ollama model (documented decision): `llama3.2` is stock Ollama's
/// small default — a ~1.2 GB GGUF that answers well on a laptop CPU and
/// matches the project's "free, no cloud" constraint (the model runs on the
/// user's own machine; nothing leaves the LAN). Change this constant to
/// switch models; the endpoint is Ollama's default localhost:11434
/// (`OllamaClient::new`; `with_base_url` exists in pdf_core::ai if a remote
/// host is needed later).
const CHAT_MODEL: &str = "llama3.2";

/// Budget for the page context fed to the model (characters, per the
/// `chunk_pages` policy in pdf_core::ai): a normal page fits easily; the cap
/// only guards a pathological dense page from overflowing the model's context
/// window. 6_000 chars ≈ 1.5k tokens — ample for one page's content.
const CHAT_MAX_CONTEXT_CHARS: usize = 6_000;

/// System prompt for the chat model: answers are grounded in the visible
/// page's context and stay in Spanish (the app's language).
const CHAT_SYSTEM_PROMPT: &str = "Eres un asistente que responde preguntas sobre el PDF abierto usando únicamente el contexto de la página visible. Responde en español, de forma concisa.";

fn main() -> eframe::Result<()> {
    let initial_pdf = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PDFLector",
        options,
        Box::new(move |cc| {
            App::new(cc, initial_pdf)
                .map(|app| -> Box<dyn eframe::App> { Box::new(app) })
                .map_err(|e| -> Box<dyn std::error::Error + Send + Sync> { e.into() })
        }),
    )
}

/// A stroke currently being drawn (Fase 3 draw mode): the page it belongs to
/// and the points captured so far, in page coordinates. Lives only while the
/// primary button is down; `App::commit_stroke` stores it in the annotation
/// set when the drag ends.
struct ActiveStroke {
    page_idx: usize,
    points: Vec<(f32, f32)>,
}

/// The active pointer tool (Fase 3.5). `Scroll` is the default: the
/// ScrollArea behaves normally (drag scrolls). The other three take over the
/// primary-button drag/click over a page and disable the ScrollArea's
/// drag-to-scroll while active (wheel and pinch still scroll).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolMode {
    /// Default: no capture, drag scrolls the document.
    Scroll,
    /// Fase 3: a drag captures a freehand `Stroke` (`handle_draw_input`).
    Draw,
    /// Fase 3.5: a drag captures a text `Highlight` (`handle_highlight_input`).
    Highlight,
    /// Fase 3.5: a click opens a small input that commits a `TextNote`
    /// anchored at the click (`handle_note_input`).
    Note,
}

/// A highlight drag in progress (Fase 3.5): the page it belongs to and the
/// drag's start/end points in page coordinates. Lives only while the primary
/// button is down; `App::commit_highlight` spawns the text-extraction worker
/// when the drag ends. The normalized rect is painted live while dragging.
struct ActiveHighlight {
    page_idx: usize,
    start: (f32, f32),
    end: (f32, f32),
}

/// A highlight whose per-line rects are still being computed on the
/// background thread. The entry is kept only to paint the fallback drag rect
/// while in flight; the reply carries everything needed to commit.
struct PendingHighlight {
    /// Echo of `highlight_seq` at commit time; matches the worker's reply.
    id: u64,
    page_idx: usize,
    /// The drag rect, painted as an immediate preview until the per-line
    /// rects arrive (and used as fallback when no text line matches).
    rect: Rect,
}

/// One completed highlight computation (see `highlight_worker`). `rects` are
/// per-line rects in page coordinates; `Err` is a user-visible Spanish
/// message (e.g. "no se pudo abrir el documento").
struct HighlightReply {
    id: u64,
    page_idx: usize,
    /// The drag rect handed to the worker — the fallback when `rects` is
    /// empty (drag over a spot with no extractable text) or errored.
    drag: Rect,
    rects: Result<Vec<Rect>, String>,
}

/// The floating text input for a note being written (Fase 3.5): the page and
/// anchor (page coordinates) are fixed at click time, `pos` is the screen
/// position where the input floats (so it appears right at the click),
/// `text` the draft. Enter or a click elsewhere commits, Escape cancels.
struct NoteInput {
    page_idx: usize,
    anchor: (f32, f32),
    pos: egui::Pos2,
    text: String,
    /// Id of the existing note being edited, `None` when creating a new one.
    /// `commit_note` replaces the annotation with this id (same anchor, new
    /// text) instead of adding a fresh note, so editing from the annotations
    /// panel reuses the exact same floating input as creation.
    editing: Option<u64>,
    /// Whether the TextEdit has already been given focus (first frame only,
    /// so the user can type immediately without a second click).
    focus_requested: bool,
    /// When the input opened; click-elsewhere only commits after this grace
    /// period, so the very click that *opened* the input (whose release can
    /// arrive a frame later, at the same position, outside the area) cannot
    /// commit an empty note immediately.
    opened_at: Instant,
}

/// Outcome of one chat turn: `Ok(answer)` is the assistant's reply,
/// `Err(message)` a user-visible error (e.g. "Ollama no accesible en
/// localhost:11434"). The message is already in Spanish — see `chat_worker`.
type ChatReply = Result<String, String>;

/// What a background export produces (Fase 3-4 toolbar buttons): a Markdown
/// sidecar or an annotated PDF copy, both written next to the source PDF.
#[derive(Clone, Copy)]
enum ExportKind {
    /// `export_markdown_to_file(<pdf>.md, …)` — text + annotation quotes.
    Markdown,
    /// `export_pdf_annotated(<pdf>, set, <pdf>.annotated.pdf)` — standard
    /// PDF annotations legible in any reader.
    Pdf,
}

/// Outcome of one background export: `Ok(path)` of the written file, or a
/// user-visible Spanish error message (see `export_worker`).
type ExportResult = Result<PathBuf, String>;

/// One Q/A turn in the chat history (Fase 5): the question, the page its
/// context was extracted from, and the answer — `None` while the background
/// thread is still talking to Ollama (the "…" state).
struct ChatEntry {
    page: usize,
    question: String,
    answer: Option<ChatReply>,
}

/// The document and all rendering live in `pdf_core::prefetch::Prefetcher`'s
/// worker thread (`mupdf::Document` is not `Send`, so the document is created
/// *inside* that thread — see `pdf_core::prefetch`). The app only calls its
/// non-blocking methods from the UI thread (`request`, `get_page` with
/// `try_recv`); it never renders a page itself (AGENTS.md §4.6).
struct App {
    prefetcher: Option<Prefetcher<MupdfEngine>>,
    /// Page sizes in PDF points (1/72"), index = page index. Read once at open
    /// from a throwaway UI-thread document (`Document::page_size` is metadata,
    /// no pixels); drives the scroll layout.
    page_sizes: Vec<(f32, f32)>,
    /// Prefix sums of page heights in points, `cum_heights[p]` = offset of
    /// page p's top. Monotonic → the visible window is a binary search.
    cum_heights: Vec<f32>,
    /// GPU textures, kept only for pages near the visible window (virtualized
    /// scroll: pages far outside are never painted nor uploaded). Dropping the
    /// handle frees the GPU memory.
    textures: HashMap<usize, egui::TextureHandle>,
    /// In-flight `get_page` receivers per page, polled with `try_recv`.
    pending: HashMap<usize, Receiver<Option<Bitmap>>>,
    /// Last `get_page` attempt per page — throttle against flooding the actor.
    last_get: HashMap<usize, Instant>,
    /// Ladder level of the current load cycle. When the zoom crosses a level
    /// boundary, old-level textures are dead weight (each level is a distinct
    /// render): all textures/pending are discarded and a fresh request at the
    /// new level takes over.
    request_level: Option<u32>,
    /// Last `(Viewport, level)` submitted to the prefetcher; `request()` is
    /// only sent when one of the two changes (the actor replaces its whole
    /// wishlist per request, so identical submissions would be wasted work).
    last_request: Option<(Viewport, u32)>,
    /// When `last_request` was submitted; gates `request()` to
    /// `REQUEST_INTERVAL` so fast scrolls do not flood the actor's queue.
    last_request_at: Option<Instant>,
    /// Bumped on every open; salted into the ScrollArea id so egui forgets
    /// the previous document's scroll position (fresh start at the top).
    open_counter: u64,
    status: String,
    doc_name: Option<String>,
    page_count: u32,
    /// Continuous zoom factor, 1.0 = 100% (1 PDF point = 1 logical pixel).
    zoom: f32,
    /// Dark mode (Fase 2): inverts the pages at texture upload and switches
    /// egui to the dark theme. Persisted in eframe's storage (see
    /// `KEY_DARK_MODE`); pdf_core's cache always keeps the normal bitmaps.
    dark_mode: bool,
    /// Debug overlay toggle (Fase 1): frame-time p95, RSS and cache counters.
    show_debug: bool,
    /// In-memory annotation set (Fase 3): vector annotations in page
    /// coordinates, drawn as an overlay layer over the page textures
    /// (AGENTS.md §4.3 — never rasterized into the cached bitmap). SQLite
    /// persistence is a later task; the set lives for the session only and
    /// resets on open.
    annotations: AnnotationSet,
    /// Active pointer tool (Fase 3 / 3.5): Draw, Highlight and Note capture
    /// the primary-button drag/click over a page; Scroll lets the ScrollArea
    /// drag-scroll normally (drag-to-scroll is disabled while a tool is
    /// active, see `update`).
    tool: ToolMode,
    /// The stroke being drawn, or `None` when the button is up.
    active_stroke: Option<ActiveStroke>,
    /// The highlight drag in progress, or `None` when the button is up.
    active_highlight: Option<ActiveHighlight>,
    /// Highlights whose per-line rects are still being computed on the
    /// background thread (painted as their drag rect meanwhile).
    pending_highlights: Vec<PendingHighlight>,
    /// Monotonic tag for `pending_highlights` / `HighlightReply` matching.
    highlight_seq: u64,
    /// Receiver of the background highlight thread's reply, polled with
    /// `try_recv` in `update` (same pattern as `chat_rx`/`export_rx`).
    highlight_rx: Option<Receiver<HighlightReply>>,
    /// The floating note input, `None` while no note is being written.
    note_input: Option<NoteInput>,
    /// Annotations panel toggle (toolbar "Annotations").
    annot_panel_open: bool,
    /// Id of the annotation last jumped to from the panel (visual highlight
    /// in the list); `None` = nothing selected yet.
    annot_selected: Option<u64>,
    /// Page to jump-and-center on the next scroll frame (set by the panel,
    /// consumed once in `scroll_body` via `ui.scroll_to_rect`).
    pending_jump: Option<usize>,
    /// Recently opened PDFs, most recent first (capped at `RECENTS_MAX`),
    /// persisted in eframe's storage (`KEY_RECENTS`).
    recent_pdfs: Vec<PathBuf>,
    /// Set when `recent_pdfs` changed; flushed to storage on the next
    /// `update` (where `frame.storage_mut()` is available — `App::new` only
    /// has an immutable `cc.storage`).
    recents_dirty: bool,
    /// Rolling window of the last active frame durations (see
    /// `ACTIVE_FRAME_MAX`); `p95()` drives the overlay's headline number.
    frame_timer: FrameTimer,
    /// When the last `update` started; the next frame's duration is measured
    /// from it.
    frame_start: Option<Instant>,
    /// Duration of the last completed active frame, shown as "frame:" in the
    /// overlay next to the p95.
    last_frame: Duration,
    // Fase 5 — AI chat panel (see `submit_chat`/`chat_worker` for the
    // background-thread flow; the UI thread never waits on Ollama).
    /// Toggle for the chat panel (toolbar "💬 Chat" button).
    chat_open: bool,
    /// The one-line question being typed in the panel.
    chat_input: String,
    /// Q/A history shown in the panel; the last entry's `answer` is `None`
    /// while its request is in flight.
    chat_history: Vec<ChatEntry>,
    /// A question is in flight: guards against a second `Ask` until the reply
    /// lands and keeps the "…" state visible.
    chat_busy: bool,
    /// Receiver for the background chat thread's reply, polled with
    /// `try_recv` (same pattern as `pending`/`poll_pending`).
    chat_rx: Option<Receiver<ChatReply>>,
    /// First page visible in the scroll viewport (updated each frame in
    /// `scroll_body`); the chat context is extracted from this page.
    current_page: usize,
    /// Path of the open document. The chat thread re-opens the PDF itself to
    /// extract text (MuPDF's context is per-thread TLS, so the prefetcher's
    /// document can never leave its worker thread — see `App::open`).
    doc_path: Option<PathBuf>,
    // Fase 3-4 closure — persistence, export, sync (see the module docs).
    /// SQLite sidecar store for the open document's annotations: opened
    /// from the PDF path at open (`store::sidecar_path`) and kept alive
    /// while the document is open — saves on stroke commit / Clear write
    /// through it (`save_annotations`). Dropped on open; re-opened on sync
    /// reload (see `reload_annotations`).
    store: Option<AnnotationStore>,
    /// Fase 4 sync: owns the `notify` watch over the sidecar. Kept here so
    /// it stays alive — dropping it stops the OS watch and joins the
    /// debounce thread. Its events only land in `sync_rx`; App state is
    /// never mutated from the watcher's background thread.
    watcher: Option<AnnotationWatcher>,
    /// Receiver of the watcher's reload triggers, polled with `try_recv` in
    /// `update` (same pattern as `pending`/`chat_rx`): the watcher callback
    /// runs on a background thread and only sends an empty message here;
    /// the UI thread reloads the sidecar and repaints (`reload_annotations`).
    sync_rx: Option<Receiver<()>>,
    /// Receiver of the background export thread's result (Fase 3-4
    /// export), polled with `try_recv` in `update`. `Some` also means an
    /// export is in flight — the toolbar buttons are disabled meanwhile.
    export_rx: Option<Receiver<ExportResult>>,
    /// User-visible note appended to the toolbar status line: sidecar load
    /// failure, sync reload error or export result. `None` = nothing to
    /// report.
    status_note: Option<String>,
}

impl App {
    fn new(
        cc: &eframe::CreationContext<'_>,
        initial_pdf: Option<PathBuf>,
    ) -> pdf_core::Result<Self> {
        // Validate the one-time global MuPDF init up front (surfaces engine
        // init errors instead of panicking); each document gets its own engine
        // moved into the prefetcher's worker thread.
        MupdfEngine::new()?;
        let mut app = Self {
            prefetcher: None,
            page_sizes: Vec::new(),
            cum_heights: Vec::new(),
            textures: HashMap::new(),
            pending: HashMap::new(),
            last_get: HashMap::new(),
            request_level: None,
            last_request: None,
            last_request_at: None,
            open_counter: 0,
            status: "no document".to_string(),
            doc_name: None,
            page_count: 0,
            zoom: 1.0,
            dark_mode: false,
            show_debug: false,
            annotations: AnnotationSet::new(),
            tool: ToolMode::Scroll,
            active_stroke: None,
            active_highlight: None,
            pending_highlights: Vec::new(),
            highlight_seq: 0,
            highlight_rx: None,
            note_input: None,
            annot_panel_open: false,
            annot_selected: None,
            pending_jump: None,
            recent_pdfs: Vec::new(),
            recents_dirty: false,
            frame_timer: FrameTimer::new(),
            frame_start: None,
            last_frame: Duration::ZERO,
            chat_open: false,
            chat_input: String::new(),
            chat_history: Vec::new(),
            chat_busy: false,
            chat_rx: None,
            current_page: 0,
            doc_path: None,
            store: None,
            watcher: None,
            sync_rx: None,
            export_rx: None,
            status_note: None,
        };
        // Restore the persisted preferences. `cc.storage` is `None` only when
        // eframe's persistence backend is unavailable; then the light theme
        // (egui's default) and an empty recents list stay.
        if let Some(storage) = cc.storage {
            app.dark_mode = storage
                .get_string(KEY_DARK_MODE)
                .is_some_and(|v| v == "true");
            // Recents are stored as a RON `Vec<PathBuf>` (see `save`). A
            // corrupt/unparseable value (e.g. a format change across egui
            // upgrades) degrades to an empty list — never a crash.
            app.recent_pdfs =
                eframe::get_value::<Vec<PathBuf>>(storage, KEY_RECENTS).unwrap_or_default();
        }
        app.apply_theme(&cc.egui_ctx);
        if let Some(path) = initial_pdf {
            match app.open(path.clone()) {
                Ok(()) => app.push_recent(path),
                Err(e) => app.status = format!("error opening: {e}"),
            }
        }
        Ok(app)
    }

    /// Opens `path`. Blocks only for a short one-time handshake (prefetcher
    /// init + reading page sizes — no pixels are rendered here); everything
    /// else is async through the prefetcher.
    fn open(&mut self, path: PathBuf) -> pdf_core::Result<()> {
        // Clean slate: no stale state from a previous document may survive an
        // open (UI state consistency).
        self.prefetcher = None; // drops the old worker (joins its thread)
        self.page_sizes.clear();
        self.cum_heights.clear();
        self.textures.clear();
        self.pending.clear();
        self.last_get.clear();
        self.request_level = None;
        self.last_request = None;
        self.last_request_at = None;
        self.doc_name = None;
        self.page_count = 0;
        self.zoom = 1.0;
        // Annotations are per-document (Fase 3): opening a new PDF starts
        // from a clean slate — no stale strokes may survive into the new
        // document (UI state consistency, as above). The new document's
        // sidecar is loaded below (`open_annotation_store`).
        self.annotations = AnnotationSet::new();
        self.tool = ToolMode::Scroll;
        self.active_stroke = None;
        self.active_highlight = None;
        self.pending_highlights.clear();
        self.highlight_rx = None;
        self.note_input = None;
        self.annot_panel_open = false;
        self.annot_selected = None;
        self.pending_jump = None;
        // Chat context is per-document (the visible page's text): a new PDF
        // starts with an empty history and a closed panel — no stale Q/A may
        // survive into the new document (UI state consistency, as above).
        self.chat_open = false;
        self.chat_input.clear();
        self.chat_history.clear();
        self.chat_busy = false;
        self.chat_rx = None;
        self.current_page = 0;
        self.doc_path = None;
        // Fase 3-4: the annotation sidecar store and the sync watcher are
        // per-document — a new open drops the old store's connection and
        // joins the old watcher's debounce thread (clean slate, as above).
        self.store = None;
        self.watcher = None;
        self.sync_rx = None;
        self.export_rx = None;
        self.status_note = None;
        self.status = format!("{} — opening…", path.display());

        // The renderer: `Prefetcher::open` spawns its worker, opens the
        // document and builds the byte-bounded cache inside it (the MuPDF
        // context is per-thread TLS). The app therefore needs no thread of its
        // own; it just calls prefetcher methods from the UI thread.
        let engine = MupdfEngine::new()?;
        let prefetcher = Prefetcher::open(engine, &path, BYTE_BUDGET)?;

        // Layout metadata: a second, throwaway UI-thread document is opened
        // only to read page sizes (metadata, never rendered — MuPDF keeps a
        // per-thread context, so this document never crosses a thread
        // boundary). A failing page size is unexpected for a document that
        // already opened; falling back to A4 keeps one corrupt page from
        // breaking the whole layout.
        let meta = MupdfEngine::new()?.open(&path)?;
        let pages = meta.page_count();
        let mut sizes = Vec::with_capacity(pages as usize);
        let mut cum = Vec::with_capacity(pages as usize + 1);
        cum.push(0.0);
        for p in 0..pages {
            let (w, h) = meta.page_size(p).unwrap_or((595.0, 842.0));
            sizes.push((w, h));
            cum.push(cum[p as usize] + h);
        }

        self.prefetcher = Some(prefetcher);
        self.page_sizes = sizes;
        self.cum_heights = cum;
        self.doc_name = Some(path.display().to_string());
        self.page_count = pages;
        // New ScrollArea id → egui forgets the previous document's offset.
        self.open_counter += 1;
        // Keep the owned path for the chat thread, the export thread and
        // the sync watcher (they re-open the PDF / sidecar themselves;
        // `doc_name` is only a display string).
        self.doc_path = Some(path.clone());
        // Fase 3-4 persistence: load the annotations sidecar next to the PDF
        // and keep the store open for the whole document lifetime (saves on
        // stroke commit / Clear). A missing sidecar is the normal first run
        // — `AnnotationStore::open` creates the (empty) database; a failing
        // or corrupt file starts with an empty set and warns in the status
        // line instead of panicking (no panic on data errors).
        self.open_annotation_store();
        // Fase 4 sync: watch the sidecar so annotations changed on another
        // device (Syncthing replaces the file) hot-reload into the UI. The
        // watcher is kept in `self.watcher` to stay alive.
        self.mount_sync_watcher(&path);
        Ok(())
    }

    /// Applies a new continuous zoom, clamped to `ZOOM_MIN..=ZOOM_MAX`.
    ///
    /// Fast path: while the ladder level is unchanged, the held textures are
    /// simply painted at the new page size (GPU rescale) — instant response.
    /// When the level changes, `App::update` discards the old-level textures
    /// and the next frame's `request`/`get_page` cycle re-renders crisp at the
    /// new level; a placeholder shows briefly until the first bitmap lands.
    fn set_zoom(&mut self, zoom: f32) {
        if !zoom.is_finite() {
            return;
        }
        self.zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
    }

    /// Applies the current theme to egui and re-uploads the visible textures
    /// with the new mode (PLAN Fase 2 "instant toggle").
    ///
    /// No document reload and no engine re-render: dropping the textures
    /// makes `issue_gets` re-fetch the same pages from the prefetcher's LRU
    /// cache (they are already resident, normal bitmaps) and `upload_texture`
    /// applies `invert_bitmap` on the way to the GPU. `last_get` is cleared
    /// so the re-issue is not throttled by the `GET_PAGE_RETRY` gate.
    fn apply_theme(&mut self, ctx: &egui::Context) {
        ctx.set_visuals(if self.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });
        self.textures.clear();
        self.last_get.clear();
    }

    /// One virtualized frame of the scroll viewport.
    ///
    /// Layout: pages are stacked top-to-bottom, each `page_size × zoom`
    /// logical pixels tall, `PAGE_GAP` apart. Only pages intersecting the egui
    /// viewport are painted; everything else is neither drawn nor textured
    /// (virtualization). All rendering happens on the prefetcher's worker
    /// thread — here we only submit the visible window, poll `get_page`
    /// receivers and upload landed bitmaps to the GPU (AGENTS.md §4.6).
    fn scroll_body(
        &mut self,
        ui: &mut egui::Ui,
        prefetcher: &Prefetcher<MupdfEngine>,
        level: u32,
        total: usize,
        viewport: egui::Rect,
    ) {
        let content_width = ui.available_width();
        // Reserve the whole content height so the scrollbar spans the entire
        // document; pages are then painted at absolute positions inside it.
        ui.allocate_space(egui::vec2(content_width, self.total_height()));

        if total == 0 {
            return;
        }
        // Content-relative scroll offset (logical pixels), read back from the
        // egui viewport; the basis for the visible window.
        let offset = viewport.min.y;
        let now = Instant::now();
        let (first, last) = self.visible_pages(offset, viewport.height());
        // Chat context (Fase 5): the panel asks about the first visible page;
        // updated every frame so the question always refers to what the user
        // is looking at.
        self.current_page = first;
        let vp = Viewport {
            first_visible_page: first,
            visible_count: last - first + 1,
        };

        // Submit the window only when it changed (visible pages first, then
        // `PREFETCH_RADIUS_MAX` neighbours): each request replaces the actor's
        // whole wishlist, so identical resubmissions would be wasted work. The
        // `REQUEST_INTERVAL` gate keeps a fast scroll from queuing one stale
        // wishlist per viewport change (the worker drains FIFO without
        // cancellation); the pipeline stays alive afterwards via the repaint
        // requests below, so the pending submission always goes through.
        let request_due = self
            .last_request_at
            .is_none_or(|t| now.duration_since(t) >= REQUEST_INTERVAL);
        if self.last_request != Some((vp, level)) && request_due {
            let radius = self.request_radius(vp, level);
            prefetcher.request(&vp, total, radius, level);
            self.last_request = Some((vp, level));
            self.last_request_at = Some(now);
        }

        // Texture pipeline: collect landed bitmaps, then ask the prefetcher
        // for visible pages still missing a texture (throttled).
        self.poll_pending(ui.ctx());
        self.issue_gets(prefetcher, level, first, last);
        // Free GPU memory for pages far outside the visible window; egui
        // releases the texture when the handle is dropped. The worker's LRU
        // cache still holds the CPU bitmap for quick re-entry.
        self.prune(first, last);

        // Fase 3/3.5: the annotation overlay. `content_origin` (top-left of
        // the scroll content, screen space) is computed once and shared by the
        // pointer capture and both paint passes, so a stroke/highlight lands
        // exactly where the cursor was at any zoom — same rects, same zoom
        // factor (see `page_rect`/`screen_to_page`). The capture runs before
        // painting so the live stroke/highlight shows the current frame's
        // pointer position.
        let content_origin = ui.max_rect().min;
        // Panel "jump to page": consumed once per request. The rect is in the
        // scroll content's coordinate space, so `scroll_to_rect` (which
        // adjusts the parent ScrollArea's offset via the pass state, see the
        // egui docs) centers the page in the viewport; ScrollArea then
        // animates to the target.
        if let Some(page) = self.pending_jump.take()
            && page < self.page_count as usize
        {
            let rect = self.page_rect(page, content_origin, content_width);
            ui.scroll_to_rect(rect, Some(egui::Align::Center));
        }
        match self.tool {
            ToolMode::Scroll => {}
            ToolMode::Draw => {
                self.handle_draw_input(ui, content_origin, content_width, first, last)
            }
            ToolMode::Highlight => {
                self.handle_highlight_input(ui, content_origin, content_width, first, last);
            }
            ToolMode::Note => {
                self.handle_note_input(ui, content_origin, content_width, first, last)
            }
        }
        self.paint_pages(ui, content_width, first, last, content_origin);
        self.paint_annotations(ui, content_origin, content_width, first, last);

        // Keep the UI alive while bitmaps are still landing (without input
        // events egui would otherwise go idle and the placeholders would
        // stay). Pages that never become resident (render error / eviction)
        // are re-queried slowly instead of busy-looping at 60 fps.
        if !self.pending.is_empty() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        } else if (first..=last).any(|p| !self.textures.contains_key(&p)) {
            ui.ctx().request_repaint_after(Duration::from_millis(250));
        }
    }

    /// Polls every in-flight `get_page` receiver with `try_recv` (never blocks
    /// the UI thread) and uploads landed bitmaps to GPU textures.
    fn poll_pending(&mut self, ctx: &egui::Context) {
        if self.pending.is_empty() {
            return;
        }
        let mut uploads: Vec<(usize, Bitmap)> = Vec::new();
        let mut done: Vec<usize> = Vec::new();
        let mut disconnected = false;
        for (&page, rx) in &self.pending {
            match rx.try_recv() {
                Ok(Some(bmp)) => {
                    uploads.push((page, bmp));
                    done.push(page);
                }
                // Not resident (yet): drop the receiver; `issue_gets` retries
                // after its throttle. The `request()` already asked for it.
                Ok(None) => done.push(page),
                // Worker still busy (it renders synchronously in its loop):
                // keep polling next frame.
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    done.push(page);
                    disconnected = true;
                }
            }
        }
        for page in done {
            self.pending.remove(&page);
        }
        for (page, bmp) in uploads {
            self.upload_texture(ctx, page, bmp);
        }
        if disconnected {
            self.status = "render worker stopped unexpectedly".to_string();
        }
    }

    /// Asks the prefetcher for a `get_page` receiver for every page near the
    /// visible window that still lacks a texture, throttled per page: a page
    /// that is not resident yet is retried at `GET_PAGE_RETRY` instead of
    /// every frame, so a busy worker is not flooded with snapshot commands.
    fn issue_gets(
        &mut self,
        prefetcher: &Prefetcher<MupdfEngine>,
        level: u32,
        first: usize,
        last: usize,
    ) {
        let now = Instant::now();
        let page_count = self.page_count as usize;
        let lo = first.saturating_sub(TEXTURE_MARGIN);
        let hi = last.saturating_add(TEXTURE_MARGIN).min(page_count - 1);
        for page in lo..=hi {
            if self.textures.contains_key(&page) || self.pending.contains_key(&page) {
                continue;
            }
            let throttled = self
                .last_get
                .get(&page)
                .is_some_and(|t| now.duration_since(*t) < GET_PAGE_RETRY);
            if throttled {
                continue;
            }
            let rx = prefetcher.get_page(page, level);
            self.pending.insert(page, rx);
            self.last_get.insert(page, now);
        }
    }

    /// Uploads a landed bitmap to a GPU texture. The texture's pixel size is
    /// `page_size × 2^level` (the prefetcher rendered at the ladder level);
    /// the paint loop draws it into the `page_size × zoom` logical rect, so
    /// the GPU rescales — never an upscale of a blurry buffer.
    fn upload_texture(&mut self, ctx: &egui::Context, page: usize, bmp: Bitmap) {
        // Dark mode inverts only the *uploaded* copy: pdf_core's cache always
        // keeps the normal page bitmaps (PLAN Fase 2 "caché coherente" —
        // mixing inverted and normal pages in the LRU would poison it on
        // theme toggle). `invert_bitmap` is a pure copy (per-pixel RGBA), so
        // toggling re-uploads the visible textures from the cache without any
        // engine re-render.
        let bmp = if self.dark_mode {
            invert_bitmap(&bmp)
        } else {
            bmp
        };
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [bmp.width as usize, bmp.height as usize],
            &bmp.data,
        );
        let tex = ctx.load_texture(format!("page-{page}"), image, egui::TextureOptions::LINEAR);
        self.textures.insert(page, tex);
    }

    /// Frees GPU memory (and stale receivers) for pages far outside the
    /// visible window. Textures keep a ±`TEXTURE_MARGIN` margin so the edges
    /// of the window do not thrash the texture cache on small scroll jitter;
    /// the worker's LRU cache still holds the CPU bitmaps.
    fn prune(&mut self, first: usize, last: usize) {
        let lo = first.saturating_sub(TEXTURE_MARGIN);
        let hi = last.saturating_add(TEXTURE_MARGIN);
        self.textures.retain(|&page, _| (lo..=hi).contains(&page));
        self.pending.retain(|&page, _| (lo..=hi).contains(&page));
        self.last_get.retain(|&page, _| (lo..=hi).contains(&page));
    }

    /// Screen rect of page `page`, in logical pixels — the single source of
    /// truth for page placement, shared by painting, the draw-mode capture
    /// and the annotation overlay.
    ///
    /// # Screen ↔ page-points transform
    ///
    /// A page of `(w, h)` PDF points is always drawn into the `w×zoom` ×
    /// `h×zoom` logical-pixel rect, horizontally centred in the scroll
    /// content and stacked under the previous pages (`cum_heights` prefix
    /// sums) plus `PAGE_GAP` per gap. `content_origin` is `ui.max_rect().min`
    /// (the top-left of the scroll content, already translated by the scroll
    /// offset), so `pos = origin + (page_offset × zoom)` in screen space.
    ///
    /// The texture itself is `page_size × 2^level` pixels (`level =
    /// scale_level_for_zoom(zoom × ppp)`, see `upload_texture`); the GPU
    /// downscales it into this rect, so displayed = tex_size × zoom / 2^level
    /// = page_size × zoom. The level/ppp only set the texture's pixel
    /// density (the crispness guarantee), never the on-screen rect — which is
    /// why they do not enter the cursor transform below.
    fn page_rect(&self, page: usize, content_origin: egui::Pos2, content_width: f32) -> egui::Rect {
        let (w, h) = self.page_sizes[page];
        let size = egui::vec2(w * self.zoom, h * self.zoom);
        let pos = egui::pos2(
            content_origin.x + (content_width - size.x) * 0.5,
            content_origin.y + self.cum_heights[page] * self.zoom + page as f32 * PAGE_GAP,
        );
        egui::Rect::from_min_size(pos, size)
    }

    /// Screen → page-points transform: the exact inverse of `page_rect`'s
    /// placement. Since 1 logical pixel = `1/zoom` PDF points on the page, a
    /// cursor at `pos` maps to `(pos − page_rect.min) / zoom` (see
    /// `page_rect` for why the texture level does not appear). The same
    /// formula — same `page_rect`, same `zoom` — is used for capture and for
    /// painting, so the stroke captured from the cursor lands exactly where it
    /// was drawn, at any zoom.
    fn screen_to_page(&self, pos: egui::Pos2, page_rect: egui::Rect) -> (f32, f32) {
        let rel = pos - page_rect.min;
        (rel.x / self.zoom, rel.y / self.zoom)
    }

    /// The page whose rect contains screen position `pos`, among the visible
    /// pages (`first..=last` covers the whole viewport); `None` when the
    /// cursor is over a page gap, the scrollbar margin or outside the scroll
    /// content (e.g. over the toolbar or a floating window).
    fn page_at(
        &self,
        pos: egui::Pos2,
        content_origin: egui::Pos2,
        content_width: f32,
        first: usize,
        last: usize,
    ) -> Option<usize> {
        (first..=last).find(|&p| {
            self.page_rect(p, content_origin, content_width)
                .contains(pos)
        })
    }

    /// Draw mode (Fase 3): turns a primary-button drag over a page into a
    /// freehand `Stroke` in page coordinates.
    ///
    /// Lifecycle: on press the page under the cursor is fixed and the first
    /// point captured; every later frame while the button is down appends the
    /// cursor position (throttled to `MIN_POINT_DIST_PX`); on release (or
    /// when the pointer leaves the window mid-drag, or a new press interrupts
    /// a dangling stroke) the capture is committed via `commit_stroke`. A
    /// fast click — press and release between two frames — yields a
    /// degenerate 1-point stroke that `Stroke::new` discards.
    fn handle_draw_input(
        &mut self,
        ui: &egui::Ui,
        content_origin: egui::Pos2,
        content_width: f32,
        first: usize,
        last: usize,
    ) {
        // Visual feedback that a drag will draw instead of scroll.
        if ui.rect_contains_pointer(ui.max_rect()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let (pos, pressed, down, released) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.latest_pos(),
                p.button_pressed(egui::PointerButton::Primary),
                p.button_down(egui::PointerButton::Primary),
                p.button_released(egui::PointerButton::Primary),
            )
        });

        // The pointer left the window mid-stroke: egui may never deliver the
        // release event, so commit what was captured so far.
        let Some(pos) = pos else {
            if let Some(stroke) = self.active_stroke.take() {
                self.commit_stroke(stroke);
            }
            return;
        };

        // Release always ends the active stroke first. In a fast click both
        // the press and the release arrive in the same frame: the commit is a
        // no-op there and the `pressed && !released` guard below keeps the
        // click from starting a new 1-point stroke.
        if released && let Some(stroke) = self.active_stroke.take() {
            self.commit_stroke(stroke);
        }

        if pressed && !released {
            // A new press also ends any dangling stroke first (self-healing
            // when a release was missed, e.g. the window lost focus mid-drag).
            if let Some(stroke) = self.active_stroke.take() {
                self.commit_stroke(stroke);
            }
            // Start a stroke only when the press lands inside a page (not on
            // the toolbar, the scrollbar thumb or a page gap).
            if let Some(page) = self.page_at(pos, content_origin, content_width, first, last) {
                let rect = self.page_rect(page, content_origin, content_width);
                self.active_stroke = Some(ActiveStroke {
                    page_idx: page,
                    points: vec![self.screen_to_page(pos, rect)],
                });
            }
        } else if down {
            // Append the cursor position. The stroke's page is fixed at press
            // time, so points keep mapping to that page even when the cursor
            // briefly leaves it (out-of-page segments are clipped at paint
            // time, see `paint_annotations`).
            let Some(s) = &self.active_stroke else {
                return;
            };
            let page_idx = s.page_idx;
            let last = s.points.last().copied();
            let rect = self.page_rect(page_idx, content_origin, content_width);
            let pt = self.screen_to_page(pos, rect);
            // Skip sub-pixel motion: `MIN_POINT_DIST_PX` in screen pixels,
            // converted to page units by the zoom.
            let min_dist = MIN_POINT_DIST_PX / self.zoom;
            let far_enough = last.is_none_or(|(lx, ly)| {
                let dx = pt.0 - lx;
                let dy = pt.1 - ly;
                dx * dx + dy * dy >= min_dist * min_dist
            });
            if far_enough && let Some(stroke) = &mut self.active_stroke {
                stroke.points.push(pt);
            }
        }
    }

    /// Stores a finished capture as a `Stroke` annotation in the page's
    /// bucket. A degenerate polyline (fewer than 2 points — a click without
    /// drag) is discarded by `Stroke::new`.
    fn commit_stroke(&mut self, active: ActiveStroke) {
        if let Some(stroke) = Stroke::new(active.points, STROKE_WIDTH_PT, STROKE_COLOR) {
            self.annotations
                .add(active.page_idx, Annotation::Stroke(stroke));
            // Fase 3-4: persist right away (synchronous, on the UI thread —
            // the save is a small single-transaction rewrite, far below the
            // frame budget, and it runs on user action, never per frame). No
            // debounce: a quick close right after drawing would lose the
            // last stroke.
            self.save_annotations();
        }
    }

    /// Highlight tool (Fase 3.5): turns a primary-button drag over a page
    /// into a text `Highlight`. Lifecycle mirrors `handle_draw_input`: the
    /// page is fixed at press, the end point tracks the cursor while down, and
    /// release commits via `commit_highlight`. A fast click yields a
    /// zero-size rect that `commit_highlight` discards (a degenerate highlight
    /// is useless).
    fn handle_highlight_input(
        &mut self,
        ui: &egui::Ui,
        content_origin: egui::Pos2,
        content_width: f32,
        first: usize,
        last: usize,
    ) {
        // Visual feedback that a drag will highlight instead of scroll.
        if ui.rect_contains_pointer(ui.max_rect()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let (pos, pressed, down, released) = ui.input(|i| {
            let p = &i.pointer;
            (
                p.latest_pos(),
                p.button_pressed(egui::PointerButton::Primary),
                p.button_down(egui::PointerButton::Primary),
                p.button_released(egui::PointerButton::Primary),
            )
        });

        // The pointer left the window mid-drag: egui may never deliver the
        // release event, so commit what was captured so far.
        let Some(pos) = pos else {
            if let Some(active) = self.active_highlight.take() {
                self.commit_highlight(active);
            }
            return;
        };

        // Release always ends the active drag first (same fast-click guard as
        // the draw tool: `pressed && !released` below keeps a click from
        // starting a new 1-point drag).
        if released && let Some(active) = self.active_highlight.take() {
            self.commit_highlight(active);
        }

        if pressed && !released {
            // A new press also ends any dangling drag first (self-healing
            // when a release was missed, as in `handle_draw_input`).
            if let Some(active) = self.active_highlight.take() {
                self.commit_highlight(active);
            }
            if let Some(page) = self.page_at(pos, content_origin, content_width, first, last) {
                let rect = self.page_rect(page, content_origin, content_width);
                let pt = self.screen_to_page(pos, rect);
                self.active_highlight = Some(ActiveHighlight {
                    page_idx: page,
                    start: pt,
                    end: pt,
                });
            }
        } else if down {
            // Track the cursor; the page is fixed at press time (points stay
            // in that page's coordinates even if the cursor leaves it —
            // out-of-page segments are clipped at paint time). The page id is
            // copied out first so the immutable `page_rect`/`screen_to_page`
            // borrows do not overlap the mutable `active_highlight` borrow.
            if let Some(page_idx) = self.active_highlight.as_ref().map(|a| a.page_idx) {
                let rect = self.page_rect(page_idx, content_origin, content_width);
                let end = self.screen_to_page(pos, rect);
                if let Some(active) = &mut self.active_highlight {
                    active.end = end;
                }
            }
        }
    }

    /// Commits a finished highlight drag. The per-line rects come from a
    /// background thread (`highlight_worker` — text extraction needs a
    /// per-thread MuPDF document, same pattern as `chat_worker`); meanwhile
    /// the drag rect is registered as a `PendingHighlight` so it paints as an
    /// immediate preview. When the reply lands (`update` drains
    /// `highlight_rx`) the `Highlight` is added to the set and saved — the
    /// reply carries everything, so out-of-order completions are harmless.
    fn commit_highlight(&mut self, active: ActiveHighlight) {
        // Normalize to a positive-extent rect (`Rect::new` re-anchors
        // negative w/h), so the drag direction does not matter.
        let rect = Rect::new(
            active.start.0,
            active.start.1,
            active.end.0 - active.start.0,
            active.end.1 - active.start.1,
        );
        // A click without drag is a zero-size highlight — discard.
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        // One highlight computation per flight: if a worker is already
        // extracting text, spawning a second would overwrite `highlight_rx`
        // and orphan the first worker's reply (its `PendingHighlight` entry
        // would leak and stay painted forever). The new drag is committed
        // right away with the fallback single rect instead — the user's
        // action is preserved, and the in-flight highlight still refines to
        // per-line rects when its reply lands.
        if self.highlight_rx.is_some() {
            self.annotations.add(
                active.page_idx,
                Annotation::Highlight(Highlight {
                    rects: vec![rect],
                    color: HIGHLIGHT_COLOR,
                }),
            );
            self.save_annotations();
            return;
        }
        let id = self.highlight_seq;
        self.highlight_seq = self.highlight_seq.saturating_add(1);
        self.pending_highlights.push(PendingHighlight {
            id,
            page_idx: active.page_idx,
            rect,
        });
        let (tx, rx) = channel::<HighlightReply>();
        self.highlight_rx = Some(rx);
        let page = active.page_idx;
        std::thread::spawn(move || {
            let reply = highlight_worker(&path, id, page, rect);
            // The app may have closed (dropped the receiver) while we were
            // extracting; that is fine — best-effort highlight.
            let _ = tx.send(reply);
        });
    }

    /// Note tool (Fase 3.5): a primary-button *click* over a page opens the
    /// floating note input anchored at the click (`note_input`; rendered as an
    /// `egui::Area` in `update`, where Enter/click-elsewhere commits and
    /// Escape cancels — see `commit_note`). While an input is already open,
    /// further clicks are ignored (the open input is modal-ish: commit or
    /// cancel first). The anchor is fixed in page coordinates, so the note
    /// stays glued to the page across zoom/scroll.
    fn handle_note_input(
        &mut self,
        ui: &egui::Ui,
        content_origin: egui::Pos2,
        content_width: f32,
        first: usize,
        last: usize,
    ) {
        if ui.rect_contains_pointer(ui.max_rect()) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        if self.note_input.is_some() {
            return;
        }
        let (pos, pressed) = ui.input(|i| {
            (
                i.pointer.latest_pos(),
                i.pointer.button_pressed(egui::PointerButton::Primary),
            )
        });
        if !pressed {
            return;
        }
        let Some(pos) = pos else { return };
        if let Some(page) = self.page_at(pos, content_origin, content_width, first, last) {
            let rect = self.page_rect(page, content_origin, content_width);
            let anchor = self.screen_to_page(pos, rect);
            self.open_note_input(page, anchor, pos, String::new(), None);
        }
    }

    /// Opens the floating note input (Fase 3.5): `editing = None` creates a
    /// new note at `anchor`; `Some(id)` edits the existing note with that id
    /// (its current text pre-fills the input), keeping the same anchor so the
    /// note stays glued to the page across zoom/scroll. Shared by the Note
    /// tool and the annotations panel's ✎ button — one input, two entry
    /// points. `pos` is the screen position where the input floats.
    fn open_note_input(
        &mut self,
        page_idx: usize,
        anchor: (f32, f32),
        pos: egui::Pos2,
        text: String,
        editing: Option<u64>,
    ) {
        self.note_input = Some(NoteInput {
            page_idx,
            anchor,
            pos,
            text,
            editing,
            focus_requested: false,
            opened_at: Instant::now(),
        });
    }

    /// Commits the open note input as a `TextNote` at its anchor (empty input
    /// falls back to `NOTE_DEFAULT_TEXT`) and persists through the sidecar.
    /// In edit mode the existing note is replaced instead: `AnnotationSet`
    /// has no update, so it is remove + add with the same anchor — the id
    /// changes, and the panel selection follows it.
    fn commit_note(&mut self) {
        let Some(note) = self.note_input.take() else {
            return;
        };
        let text = note.text.trim();
        let text = if text.is_empty() {
            NOTE_DEFAULT_TEXT.to_string()
        } else {
            text.to_string()
        };
        let kind = Annotation::TextNote(TextNote {
            anchor: note.anchor,
            text,
        });
        if let Some(id) = note.editing {
            // Remove first so the id space stays consistent (a deleted note
            // must not linger in the sidecar); add assigns a fresh id.
            self.annotations.remove(id);
            if let Some(new_id) = self.annotations.add(note.page_idx, kind)
                && self.annot_selected == Some(id)
            {
                self.annot_selected = Some(new_id);
            }
        } else {
            self.annotations.add(note.page_idx, kind);
        }
        self.save_annotations();
    }

    /// Records `path` as the most recently opened PDF: deduped, moved to the
    /// front, capped at `RECENTS_MAX`. The actual write to eframe's storage
    /// happens on the next `update`/`save` (only those have a `&mut
    /// Storage`), flagged by `recents_dirty`.
    fn push_recent(&mut self, path: PathBuf) {
        self.recent_pdfs.retain(|p| *p != path);
        self.recent_pdfs.insert(0, path);
        self.recent_pdfs.truncate(RECENTS_MAX);
        self.recents_dirty = true;
    }

    /// Writes the current `AnnotationSet` to the open document's sidecar
    /// (Fase 3-4). Synchronous on the UI thread: `AnnotationStore::save` is
    /// a small transaction over at most a few hundred rows, well below the
    /// frame budget (AGENTS.md §8), and it only runs on user actions (stroke
    /// commit, Clear). Failures surface in the status line; the in-memory
    /// set is never rolled back (the drawing stays visible this session).
    fn save_annotations(&mut self) {
        let Some(store) = &self.store else { return };
        if let Err(e) = store.save(&self.annotations) {
            self.status_note = Some(format!("anotaciones: no se pudo guardar el sidecar ({e})"));
        }
    }

    /// Opens the SQLite sidecar for the current document and loads its
    /// annotations (Fase 3-4). On any failure the app starts with an empty
    /// set and warns in the status line — never a panic.
    fn open_annotation_store(&mut self) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let sidecar = sidecar_path(&path);
        match AnnotationStore::open(&sidecar).and_then(|s| s.load().map(|set| (s, set))) {
            Ok((store, set)) => {
                self.store = Some(store);
                self.annotations = set;
            }
            Err(e) => {
                self.store = None;
                self.annotations = AnnotationSet::new();
                self.status_note = Some(format!(
                    "anotaciones: no se pudo leer el sidecar ({e}) — empezando vacío"
                ));
            }
        }
    }

    /// Fase 4: mounts a filesystem watch on the open document's sidecar.
    /// Syncthing is the one that copies the file between devices; this is
    /// only the local trigger — every debounced burst of changes sends a
    /// message to `sync_rx`, and `update` reloads the set on the UI thread
    /// (`reload_annotations`). The watcher is stored in `self.watcher` so it
    /// outlives this function (dropping it would stop the watch and join the
    /// debounce thread). Best-effort: a failed watch warns in the status
    /// line but never fails the open.
    fn mount_sync_watcher(&mut self, path: &Path) {
        let (tx, rx) = channel::<()>();
        match watch_annotations(path, move || {
            // Never block here: the watcher callback runs on a background
            // thread and must only wake the UI thread, which reloads.
            let _ = tx.send(());
        }) {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.sync_rx = Some(rx);
            }
            Err(e) => {
                self.sync_rx = None;
                self.status_note = Some(format!("sync: no se pudo observar el sidecar ({e})"));
            }
        }
    }

    /// Fase 4 sync: reloads the annotation set from the sidecar after the
    /// watcher reported a change (Syncthing replaced the file, or a previous
    /// local save). The store is re-opened first, on purpose: Syncthing
    /// replaces the sidecar atomically (write temp + rename), which gives
    /// the path a new inode that the old open connection would keep ignoring
    /// (it would read the *old* file). Re-opening is cheap — schema creation
    /// is `IF NOT EXISTS` (store.rs). After a reload the frame repaints so
    /// the synced strokes appear immediately.
    fn reload_annotations(&mut self, ctx: &egui::Context) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let sidecar = sidecar_path(&path);
        match AnnotationStore::open(&sidecar).and_then(|s| s.load().map(|set| (s, set))) {
            Ok((store, set)) => {
                self.store = Some(store);
                self.annotations = set;
                ctx.request_repaint();
            }
            Err(e) => {
                self.store = None;
                self.status_note = Some(format!("sync: no se pudo recargar el sidecar ({e})"));
                ctx.request_repaint();
            }
        }
    }

    /// Fase 3-4 export: spawns a detached background thread that writes the
    /// export, so the UI thread never waits (AGENTS.md §4.6). The result
    /// lands in `export_rx` (polled in `update`) and shows in the status
    /// line. The buttons are disabled while one export is in flight.
    fn start_export(&mut self, kind: ExportKind) {
        if self.export_rx.is_some() {
            return; // an export is already in flight
        }
        let Some(pdf_path) = self.doc_path.clone() else {
            return;
        };
        // Clone the set into the thread: it is small (hundreds of strokes at
        // most) and the export must see the state at click time.
        let set = self.annotations.clone();
        let (tx, rx) = channel::<ExportResult>();
        self.export_rx = Some(rx);
        std::thread::spawn(move || {
            let result = export_worker(kind, &pdf_path, &set);
            // The app may have closed or opened another document while the
            // export ran (receiver dropped): best-effort, ignore the send.
            let _ = tx.send(result);
        });
    }

    /// Paints the vector annotation layer on top of the page textures
    /// (AGENTS.md §4.3: annotations are never rasterized into the cached
    /// bitmap — this is a separate pass, drawn every frame by egui).
    ///
    /// Only the visible pages are considered (PLAN §3.4: cost proportional to
    /// visible strokes, never to the whole document) and each page's painter
    /// is clipped to its rect, so an out-of-page annotation segment (the
    /// cursor left the page mid-drag) is cut at the page edge instead of
    /// bleeding over the gaps or the neighbouring pages.
    ///
    /// Fase 3.5 additions: `Highlight`s paint as semi-transparent rects over
    /// the text, text notes paint a fixed-size marker (constant on-screen
    /// size, like the cursor) with a hover tooltip, and the in-progress
    /// highlight drag / pending highlight paint as a live preview rect. The
    /// painter is cloned so the hover interactions (`ui.interact`) can borrow
    /// the Ui mutably while the page painters stay alive.
    fn paint_annotations(
        &self,
        ui: &mut egui::Ui,
        content_origin: egui::Pos2,
        content_width: f32,
        first: usize,
        last: usize,
    ) {
        let painter = ui.painter().clone();
        for page in first..=last {
            let rect = self.page_rect(page, content_origin, content_width);
            let anns = self.annotations.for_page(page);
            let has_live_stroke = self
                .active_stroke
                .as_ref()
                .is_some_and(|s| s.page_idx == page);
            let has_live_highlight = self
                .active_highlight
                .as_ref()
                .is_some_and(|h| h.page_idx == page);
            let has_pending = self.pending_highlights.iter().any(|p| p.page_idx == page);
            if anns.is_empty() && !has_live_stroke && !has_live_highlight && !has_pending {
                continue;
            }
            // Clip the page's annotations to the page rect (and the viewport,
            // via the inherited clip).
            let clipped = painter.with_clip_rect(rect);
            for ann in &anns {
                match &ann.kind {
                    Annotation::Stroke(s) => Self::paint_stroke(&clipped, s, rect, self.zoom),
                    Annotation::Highlight(h) => {
                        let color = egui::Color32::from_rgba_unmultiplied(
                            h.color.r, h.color.g, h.color.b, h.color.a,
                        );
                        for r in &h.rects {
                            let screen = egui::Rect::from_min_size(
                                egui::pos2(
                                    rect.min.x + r.x * self.zoom,
                                    rect.min.y + r.y * self.zoom,
                                ),
                                egui::vec2(r.w * self.zoom, r.h * self.zoom),
                            );
                            clipped.rect_filled(screen, 0.0, color);
                        }
                    }
                    Annotation::TextNote(n) => {
                        // Marker: a fixed-size square centred on the anchor
                        // (page point → screen via the same transform as
                        // everything else). The hover interaction uses a
                        // slightly expanded hit area (`NOTE_HOVER_PAD`) so the
                        // small marker is easy to reach; the tooltip shows the
                        // note text while hovering.
                        let anchor_screen = egui::pos2(
                            rect.min.x + n.anchor.0 * self.zoom,
                            rect.min.y + n.anchor.1 * self.zoom,
                        );
                        let marker = egui::Rect::from_center_size(
                            anchor_screen,
                            egui::vec2(NOTE_MARKER_PX, NOTE_MARKER_PX),
                        );
                        clipped.rect_filled(marker, 2.0, NOTE_MARKER_COLOR);
                        clipped.rect_stroke(
                            marker,
                            2.0,
                            egui::Stroke::new(1.0_f32, NOTE_MARKER_BORDER),
                            egui::StrokeKind::Inside,
                        );
                        let hit = marker.expand(NOTE_HOVER_PAD);
                        let resp = ui.interact(
                            hit,
                            egui::Id::new(("note-marker", ann.id)),
                            egui::Sense::hover(),
                        );
                        resp.on_hover_text(n.text.as_str());
                    }
                }
            }
            // The live stroke, drawn with the same transform so it is
            // WYSIWYG while dragging.
            if let Some(active) = &self.active_stroke
                && active.page_idx == page
            {
                let color = egui::Color32::from_rgba_unmultiplied(
                    STROKE_COLOR.r,
                    STROKE_COLOR.g,
                    STROKE_COLOR.b,
                    STROKE_COLOR.a,
                );
                let points: Vec<egui::Pos2> = active
                    .points
                    .iter()
                    .map(|&(x, y)| {
                        egui::pos2(rect.min.x + x * self.zoom, rect.min.y + y * self.zoom)
                    })
                    .collect();
                if points.len() >= 2 {
                    clipped.add(egui::Shape::line(
                        points,
                        egui::Stroke::new(STROKE_WIDTH_PT * self.zoom, color),
                    ));
                }
            }
            // Live highlight drag: the normalized start→end rect, painted as
            // an immediate preview while the button is down.
            if let Some(active) = &self.active_highlight
                && active.page_idx == page
            {
                let r = Rect::new(
                    active.start.0,
                    active.start.1,
                    active.end.0 - active.start.0,
                    active.end.1 - active.start.1,
                );
                Self::paint_highlight_rect(&clipped, &r, rect, self.zoom, HIGHLIGHT_COLOR);
            }
            // Pending highlights (worker still computing per-line rects): keep
            // the drag rect visible so the user sees the highlight landing.
            for pending in self
                .pending_highlights
                .iter()
                .filter(|p| p.page_idx == page)
            {
                Self::paint_highlight_rect(
                    &clipped,
                    &pending.rect,
                    rect,
                    self.zoom,
                    HIGHLIGHT_COLOR,
                );
            }
        }
    }

    /// Paints one highlight rect: page coordinates → screen via the page
    /// rect and zoom, filled with the (semi-transparent) annotation colour.
    fn paint_highlight_rect(
        clipped: &egui::Painter,
        r: &Rect,
        page_rect: egui::Rect,
        zoom: f32,
        color: Color,
    ) {
        let screen = egui::Rect::from_min_size(
            egui::pos2(page_rect.min.x + r.x * zoom, page_rect.min.y + r.y * zoom),
            egui::vec2(r.w * zoom, r.h * zoom),
        );
        let color = egui::Color32::from_rgba_unmultiplied(color.r, color.g, color.b, color.a);
        clipped.rect_filled(screen, 0.0, color);
    }

    /// Draws one finished stroke as a polyline: page points scaled by `zoom`
    /// into screen space, width scaled the same way (a stroke of `w` points
    /// is `w × zoom` screen pixels, matching the page's `page_size × zoom`
    /// on-screen size).
    fn paint_stroke(painter: &egui::Painter, s: &Stroke, page_rect: egui::Rect, zoom: f32) {
        if s.points.len() < 2 {
            return;
        }
        let points: Vec<egui::Pos2> = s
            .points
            .iter()
            .map(|&(x, y)| egui::pos2(page_rect.min.x + x * zoom, page_rect.min.y + y * zoom))
            .collect();
        let color =
            egui::Color32::from_rgba_unmultiplied(s.color.r, s.color.g, s.color.b, s.color.a);
        painter.add(egui::Shape::line(
            points,
            egui::Stroke::new(s.width * zoom, color),
        ));
    }

    /// Paints the pages intersecting the viewport. `content_origin` is the
    /// top-left of the scroll content in screen space (see `page_rect`); the
    /// GPU draws each texture into the page's `page_size × zoom` rect, so the
    /// texture's ladder level never shows in the layout.
    fn paint_pages(
        &self,
        ui: &mut egui::Ui,
        content_width: f32,
        first: usize,
        last: usize,
        content_origin: egui::Pos2,
    ) {
        let painter = ui.painter();
        let visuals = ui.visuals();
        for page in first..=last {
            let rect = self.page_rect(page, content_origin, content_width);
            match self.textures.get(&page) {
                // Steady state: the GPU draws the texture at the current page
                // size (2^level ≥ zoom·ppp bitmap px per point downscaled into
                // `page_size × zoom` logical px — crisp, and a zoom change
                // within the same level is a free GPU rescale).
                Some(tex) => {
                    painter.image(
                        tex.id(),
                        rect,
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                // Bitmap still landing on the worker (or a page that failed to
                // render — the prefetcher swallows per-page render errors by
                // design, best-effort): cheap placeholder so the layout is
                // visible instead of a hole.
                None => {
                    painter.rect_filled(rect, 2.0, visuals.extreme_bg_color);
                    painter.text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        format!("page {}", page + 1),
                        egui::FontId::proportional(16.0),
                        visuals.weak_text_color(),
                    );
                }
            }
        }
    }

    /// Prefetch radius for a request: the largest radius whose window
    /// (visible + radius on each side) fits the byte budget at `level`, capped
    /// at `PREFETCH_RADIUS_MAX`.
    ///
    /// Why: pdf_core's LRU evicts least-recently-used entries and visible
    /// pages are rendered *first*, so when a request exceeds the budget the
    /// visible pages are the first to be evicted by their own prefetch
    /// neighbours — `get_page` would then return `None` forever (placeholders
    /// that never fill). Shrinking the radius to fit keeps the visible window
    /// resident; at very high zoom levels (one page ≥ budget) the radius
    /// becomes 0 and only the visible pages are rendered.
    fn request_radius(&self, vp: Viewport, level: u32) -> usize {
        // Page bytes at the ladder level: page points × 2^level per side × 4
        // (RGBA8). The largest page in the document is the conservative
        // estimate, so the budget holds for every page.
        let (max_w, max_h) = self
            .page_sizes
            .iter()
            .fold((0.0f32, 0.0f32), |(mw, mh), &(w, h)| (mw.max(w), mh.max(h)));
        let scale = 2.0_f32.powi(level as i32);
        let bytes_per_page = (max_w * scale) as usize * (max_h * scale) as usize * 4;
        let capacity = BYTE_BUDGET / bytes_per_page.max(1);
        let neighbours = capacity.saturating_sub(vp.visible_count) / 2;
        neighbours.min(PREFETCH_RADIUS_MAX)
    }

    /// Pages intersecting `[offset, offset + viewport_height)` (content-relative
    /// logical pixels). The layout is monotonic (`cum_heights` prefix sums), so
    /// the window is two binary searches; a partially visible page counts as
    /// visible (it must be rendered and painted).
    fn visible_pages(&self, offset: f32, viewport_height: f32) -> (usize, usize) {
        let total = self.page_count as usize;
        if total == 0 {
            return (0, 0);
        }
        let zoom = self.zoom;
        let bottom = |p: usize| self.cum_heights[p + 1] * zoom + (p + 1) as f32 * PAGE_GAP;
        let page_top = |p: usize| self.cum_heights[p] * zoom + p as f32 * PAGE_GAP;

        // First page whose bottom edge is below the scroll offset (manual
        // binary search: `partition_point` only exists on slices).
        let mut lo = 0usize;
        let mut hi = total;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if bottom(mid) <= offset {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let first = lo.min(total - 1);
        // Last page whose top edge is above the bottom of the viewport.
        let mut lo = 0usize;
        let mut hi = total;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if page_top(mid) < offset + viewport_height {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let last = lo.saturating_sub(1).clamp(first, total - 1);
        (first, last)
    }

    /// Total content height in logical pixels: page heights at the current
    /// zoom plus the gaps between them.
    fn total_height(&self) -> f32 {
        let total = self.page_count as usize;
        if total == 0 {
            return 0.0;
        }
        self.cum_heights[total] * self.zoom + (total - 1) as f32 * PAGE_GAP
    }

    /// Status line shown in the toolbar: document, page count, zoom and the
    /// worker's live cache counters (a cheap cross-thread snapshot; lets the
    /// smoke test confirm the byte-bounded LRU is doing its job while
    /// scrolling).
    fn status_line(&self) -> String {
        let mut line = match &self.prefetcher {
            Some(p) => {
                let s = p.stats_snapshot();
                format!(
                    "{} — {} pages — {:.0}% — cache {:.1} MB ({})",
                    self.doc_name.as_deref().unwrap_or("document"),
                    self.page_count,
                    self.zoom * 100.0,
                    s.current_bytes as f64 / (1024.0 * 1024.0),
                    s.entries,
                )
            }
            None => self.status.clone(),
        };
        // Fase 3-4: a transient note (sidecar warning, sync reload error,
        // export result) travels with the status line so it is visible in
        // the toolbar even while a document is open.
        if let Some(note) = &self.status_note {
            line.push_str(" — ");
            line.push_str(note);
        }
        line
    }

    /// Fase 5: submits the typed question. Appends it to the history as
    /// pending ("…") and spawns a **detached** background thread that does
    /// the whole turn — text extraction + the Ollama HTTP call (generation
    /// can take minutes) — so the UI thread never waits (AGENTS.md §4.6).
    /// The reply lands via `chat_rx`, polled with `try_recv` in `update`.
    fn submit_chat(&mut self) {
        let question = self.chat_input.trim().to_string();
        if question.is_empty() || self.chat_busy {
            return;
        }
        // Best-effort: without a document there is no context to ask about
        // (the toolbar button is disabled in that case anyway).
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let page = self.current_page;
        self.chat_input.clear();
        // Clone into the entry: `question` itself is moved into the worker
        // closure below (it is `&str` there, so the clone is the cheap copy).
        self.chat_history.push(ChatEntry {
            page,
            question: question.clone(),
            answer: None,
        });
        self.chat_busy = true;
        let (tx, rx) = channel::<ChatReply>();
        self.chat_rx = Some(rx);
        std::thread::spawn(move || {
            let reply = chat_worker(&path, page, &question);
            // The UI may have closed (dropped the receiver) while we were
            // generating; that is fine — best-effort chat.
            let _ = tx.send(reply);
        });
    }
}

/// Fase 3.5: one highlight computation on a background thread.
///
/// Re-opens the PDF on *this* thread (MuPDF's context is per-thread TLS; the
/// prefetcher's document never leaves its worker thread — same pattern as
/// `chat_worker`/`export_worker`), extracts the page's text lazily and
/// derives the per-line rects for the drag. The reply echoes `id`, `page`
/// and `drag` so the UI thread can match it against the pending preview and
/// commit without a shared queue.
fn highlight_worker(path: &Path, id: u64, page: usize, drag: Rect) -> HighlightReply {
    let rects = (|| -> pdf_core::Result<Vec<Rect>> {
        let engine = MupdfEngine::new()?;
        let doc = engine.open(path)?;
        let text = doc.text(page as u32)?;
        Ok(highlight_rects_for_drag(&text.spans, drag))
    })()
    .map_err(|e| e.to_string());
    HighlightReply {
        id,
        page_idx: page,
        drag,
        rects,
    }
}

/// Per-line highlight rects for a drag, from the page's text spans (Fase
/// 3.5 design decision — documented here so it can be revisited with data).
///
/// For every span whose bbox intersects the drag rect we emit one rect: the
/// span's full height, clipped horizontally to the drag's x-range (the
/// natural highlighter shape — selection start → end on each covered line,
/// and whole line boxes, so short drags over a line still cover the whole
/// line vertically). Spans fully outside the drag are skipped. A drag over a
/// spot with no extractable text (image scan, margin) yields an empty list;
/// the caller falls back to the single drag rect. This needs no word-level
/// data — `TextSpan` is one per stext line — so it stays the simplest
/// correct option with the current pdf_core API.
fn highlight_rects_for_drag(spans: &[TextSpan], drag: Rect) -> Vec<Rect> {
    let x0 = drag.x;
    let x1 = drag.x + drag.w;
    let y0 = drag.y;
    let y1 = drag.y + drag.h;
    let mut rects = Vec::new();
    for span in spans {
        let sx0 = span.x;
        let sy0 = span.y;
        let sx1 = span.x + span.w;
        let sy1 = span.y + span.h;
        // Strict bbox intersection (the drag must actually cover part of the
        // line, not just brush its edge).
        if sx1 <= x0 || sx0 >= x1 || sy1 <= y0 || sy0 >= y1 {
            continue;
        }
        let r0 = sx0.max(x0);
        let r1 = sx1.min(x1);
        if r1 - r0 <= 0.0 {
            continue;
        }
        rects.push(Rect::new(r0, sy0, r1 - r0, span.h));
    }
    rects
}

/// One-line summary of an annotation for the annotations panel (Fase 3.5):
/// type plus a short content hint. Note text is truncated to 40 chars
/// (char-boundary safe) so long notes stay one line.
fn annotation_summary(kind: &Annotation) -> String {
    match kind {
        Annotation::Stroke(s) => format!("✏️ stroke ({} pts)", s.points.len()),
        Annotation::Highlight(h) => format!("🖍 highlight ({} rects)", h.rects.len()),
        Annotation::TextNote(n) => {
            let text: String = n.text.chars().take(40).collect();
            format!("📝 {text}")
        }
    }
}

/// Fase 5: one full chat turn on a background thread.
///
/// Re-opens the PDF on *this* thread (MuPDF's context is per-thread TLS; the
/// prefetcher's document never leaves its worker thread — same pattern as the
/// metadata document in `App::open`), extracts the visible page's text
/// through `pdf_core::ai::chunk_pages` (lazy, AGENTS.md §4.7 — the chunk
/// policy also caps the context at `CHAT_MAX_CONTEXT_CHARS` and prefixes
/// `[págs N]` ranges so the model can cite pages), builds the prompt and
/// calls `pdf_core::ai::OllamaClient` at the default endpoint
/// (localhost:11434). Errors are mapped to user-visible Spanish messages.
fn chat_worker(path: &Path, page: usize, question: &str) -> ChatReply {
    let engine = MupdfEngine::new().map_err(|e| format!("No se pudo iniciar el motor PDF: {e}"))?;
    let doc = engine
        .open(path)
        .map_err(|e| format!("No se pudo abrir el documento: {e}"))?;
    // Text extraction + chunking: only this page is touched (Fase 5 never
    // renders nor preloads); the chunk policy keeps the prompt bounded.
    let chunks = pdf_core::ai::chunk_pages(&doc, &[page as u32], CHAT_MAX_CONTEXT_CHARS)
        .map_err(|e| format!("No se pudo extraer el texto de la página {}: {e}", page + 1))?;
    let context = chunks.join("\n");
    let prompt = format!(
        "Pregunta: {question}\n\nContexto (página {}):\n{context}",
        page + 1
    );
    let client = pdf_core::ai::OllamaClient::new(CHAT_MODEL);
    client
        .chat(CHAT_SYSTEM_PROMPT, &prompt)
        .map_err(|e| format!("Chat IA no disponible: {e}"))
}

/// Runs one export on a background thread (see `App::start_export`).
///
/// The Markdown export needs a `&dyn Document`, and the Prefetcher does not
/// expose its document — the MuPDF context is per-thread TLS, so the
/// renderer's document can never leave its worker thread (same pattern as
/// `chat_worker`). A fresh `MupdfEngine::open` is therefore created *on this
/// thread* and dropped here; the PDF export needs no document at all
/// (`export_pdf_annotated` binds MuPDF's own PdfDocument internally).
/// Errors are mapped to user-visible Spanish messages.
fn export_worker(kind: ExportKind, pdf_path: &Path, set: &AnnotationSet) -> ExportResult {
    match kind {
        ExportKind::Markdown => {
            // `<pdf>.md` next to the PDF: one small text file per document,
            // Syncthing-friendly (PLAN §3.5).
            let md_path = PathBuf::from(format!("{}.md", pdf_path.display()));
            let engine =
                MupdfEngine::new().map_err(|e| format!("no se pudo iniciar el motor PDF: {e}"))?;
            let doc = engine
                .open(pdf_path)
                .map_err(|e| format!("no se pudo abrir el documento: {e}"))?;
            pdf_core::export_markdown_to_file(&md_path, &doc, set)
                .map_err(|e| format!("export MD falló: {e}"))?;
            Ok(md_path)
        }
        ExportKind::Pdf => {
            // `<pdf>.annotated.pdf` next to the PDF (standard PDF
            // annotations, legible in any reader).
            let out = PathBuf::from(format!("{}.annotated.pdf", pdf_path.display()));
            pdf_core::export_pdf_annotated(pdf_path, set, &out)
                .map_err(|e| format!("export PDF falló: {e}"))?;
            Ok(out)
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        // Measure the last active frame (duration since the previous update).
        // The `ACTIVE_FRAME_MAX` guard skips idle gaps, which would otherwise
        // look like seconds-long frames: egui stops calling `update` when
        // nothing repaints (no input, no repaint request).
        let now = Instant::now();
        if let Some(prev) = self.frame_start
            && now.duration_since(prev) < ACTIVE_FRAME_MAX
        {
            self.last_frame = now - prev;
            self.frame_timer.push(self.last_frame);
        }
        self.frame_start = Some(now);

        // Fase 5 chat: drain the background thread's reply channel with
        // `try_recv` (never blocks the UI thread — same pattern as
        // `poll_pending`). The reply lands on the in-flight history entry; a
        // disconnected worker (thread died) is surfaced as an error so the
        // "…" can never hang forever.
        if let Some(rx) = self.chat_rx.take() {
            match rx.try_recv() {
                Ok(reply) => {
                    if let Some(last) = self.chat_history.last_mut() {
                        last.answer = Some(reply);
                    }
                    self.chat_busy = false;
                }
                Err(TryRecvError::Empty) => self.chat_rx = Some(rx),
                Err(TryRecvError::Disconnected) => {
                    if let Some(last) = self.chat_history.last_mut() {
                        last.answer =
                            Some(Err("el hilo de chat se detuvo inesperadamente".to_string()));
                    }
                    self.chat_busy = false;
                }
            }
        }
        // Fase 3.5 highlight: drain the background thread's reply channel
        // with `try_recv` (never blocks the UI thread — same pattern as
        // `chat_rx`/`export_rx`). The reply carries the drag rect and the
        // per-line rects, so it is matched to the pending preview by id and
        // committed straight away; when text extraction found no line (or
        // failed), the drag rect itself is the fallback highlight.
        if let Some(rx) = self.highlight_rx.take() {
            match rx.try_recv() {
                Ok(reply) => {
                    self.pending_highlights.retain(|p| p.id != reply.id);
                    let rects = match reply.rects {
                        Ok(rects) if !rects.is_empty() => rects,
                        Ok(_) => {
                            // Drag over a spot without extractable text: the
                            // drag rect is still a valid highlight.
                            vec![reply.drag]
                        }
                        Err(msg) => {
                            self.status_note =
                                Some(format!("highlight: no se pudo leer el texto ({msg})"));
                            vec![reply.drag]
                        }
                    };
                    self.annotations.add(
                        reply.page_idx,
                        Annotation::Highlight(Highlight {
                            rects,
                            color: HIGHLIGHT_COLOR,
                        }),
                    );
                    self.save_annotations();
                    ctx.request_repaint();
                }
                Err(TryRecvError::Empty) => self.highlight_rx = Some(rx),
                Err(TryRecvError::Disconnected) => {
                    // Worker died: commit the fallback rect so the drag is not
                    // silently lost.
                    if let Some(pending) = self.pending_highlights.pop() {
                        self.annotations.add(
                            pending.page_idx,
                            Annotation::Highlight(Highlight {
                                rects: vec![pending.rect],
                                color: HIGHLIGHT_COLOR,
                            }),
                        );
                        self.save_annotations();
                    }
                    self.status_note =
                        Some("el hilo de highlight se detuvo inesperadamente".to_string());
                    ctx.request_repaint();
                }
            }
        }
        // Fase 3-4 export: drain the background thread's result channel with
        // `try_recv` (never blocks the UI thread — same pattern as `chat_rx`
        // / `poll_pending`). Success shows the written path in the status
        // line; a dead thread is surfaced as an error so the toolbar buttons
        // cannot stay disabled forever.
        if let Some(rx) = self.export_rx.take() {
            match rx.try_recv() {
                Ok(Ok(path)) => {
                    self.status_note = Some(format!("exportado a {}", path.display()));
                }
                Ok(Err(msg)) => {
                    self.status_note = Some(format!("export fallido: {msg}"));
                }
                Err(TryRecvError::Empty) => self.export_rx = Some(rx),
                Err(TryRecvError::Disconnected) => {
                    self.status_note =
                        Some("el hilo de export se detuvo inesperadamente".to_string());
                }
            }
        }

        // Fase 4 sync: the watcher (background thread) only sends an empty
        // message per debounced burst; drain them here and reload the
        // sidecar on the UI thread (`reload_annotations`). A disconnect
        // means the watcher died — stop polling and keep the last set.
        if let Some(rx) = self.sync_rx.take() {
            let mut changed = false;
            loop {
                match rx.try_recv() {
                    Ok(()) => changed = true,
                    Err(TryRecvError::Empty) => {
                        self.sync_rx = Some(rx);
                        break;
                    }
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            if changed {
                self.reload_annotations(ctx);
            }
        }

        // Keep the UI alive while (a) a sync watcher is mounted — an idle
        // egui would never poll `sync_rx`, so a sidecar replaced by
        // Syncthing would go unnoticed until the next interaction — and (b)
        // an export is in flight (same reasoning as the chat "…" below).
        // 200 ms ≈ the watcher's debounce window (sync.rs).
        if self.sync_rx.is_some() || self.export_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(200));
        }

        // Keep the UI alive while a question is in flight: egui goes idle
        // without input events, so the "…" and the eventual reply need a
        // periodic repaint to be picked up.
        if self.chat_busy {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // Same for a highlight computation in flight: without a repaint the
        // worker's reply would never be polled once the input stops.
        if self.highlight_rx.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Persist the recents list when it changed (set by `push_recent`, e.g.
        // the initial PDF opened in `App::new` — which only has an immutable
        // `cc.storage`, so the actual write happens here on the first frame).
        if self.recents_dirty
            && let Some(storage) = frame.storage_mut()
        {
            eframe::set_value(storage, KEY_RECENTS, &self.recent_pdfs);
            storage.flush();
            self.recents_dirty = false;
        }

        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Fase 3.5: "Open PDF…" is now a menu: the file dialog plus
                // the recently opened PDFs (persisted in eframe's storage,
                // see `push_recent`/`save`). Clicking a recent opens it
                // directly, skipping the dialog.
                ui.menu_button("Open PDF…", |ui| {
                    if ui.button("Choose file…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("PDF", &["pdf"])
                            .pick_file()
                        {
                            match self.open(path.clone()) {
                                Ok(()) => self.push_recent(path),
                                Err(e) => self.status = format!("error opening: {e}"),
                            }
                        }
                        ui.close();
                    }
                    if !self.recent_pdfs.is_empty() {
                        ui.separator();
                        ui.label(egui::RichText::new("Recents:").weak());
                        // Clone: `self.open` needs `&mut self`, which would
                        // clash with borrowing the list during iteration.
                        for path in self.recent_pdfs.clone() {
                            if ui.button(path.display().to_string()).clicked() {
                                match self.open(path.clone()) {
                                    Ok(()) => self.push_recent(path),
                                    Err(e) => self.status = format!("error opening: {e}"),
                                }
                                ui.close();
                            }
                        }
                    }
                });
                let has_doc = self.prefetcher.is_some();
                if ui
                    .add_enabled(has_doc, egui::Button::new("-"))
                    .on_hover_text("Zoom out")
                    .clicked()
                {
                    self.set_zoom(self.zoom / ZOOM_STEP);
                }
                if ui
                    .add_enabled(has_doc, egui::Button::new("+"))
                    .on_hover_text("Zoom in")
                    .clicked()
                {
                    self.set_zoom(self.zoom * ZOOM_STEP);
                }
                ui.label(format!("{:.0}%", self.zoom * 100.0))
                    .on_hover_text("Ctrl+wheel or pinch: zoom — wheel: scroll");
                // Fase 3/3.5 tool toggles: while a tool is active, the primary
                // button over a page draws / highlights / notes instead of
                // scrolling (see `handle_*_input`; scroll-by-drag is disabled,
                // wheel and pinch still scroll). Exactly one tool is active at
                // a time; deselecting (Scroll) restores normal drag-scrolling.
                if has_doc {
                    ui.selectable_value(&mut self.tool, ToolMode::Draw, "✏️ Draw")
                        .on_hover_text("Draw mode: drag over a page to add a freehand stroke");
                    ui.selectable_value(&mut self.tool, ToolMode::Highlight, "🖍 Highlight")
                        .on_hover_text("Highlight mode: drag over text to highlight it");
                    ui.selectable_value(&mut self.tool, ToolMode::Note, "📝 Note")
                        .on_hover_text("Note mode: click on a page to add a text note");
                } else {
                    ui.add_enabled(false, egui::Button::selectable(false, "✏️ Draw"));
                    ui.add_enabled(false, egui::Button::selectable(false, "🖍 Highlight"));
                    ui.add_enabled(false, egui::Button::selectable(false, "📝 Note"));
                }
                ui.label(self.status_line());
                ui.separator();
                // Dark mode: instant toggle — the visible pages are
                // re-uploaded inverted from the cache (see `apply_theme` and
                // `upload_texture`; no engine re-render) and egui switches
                // visuals. The preference is flushed to eframe's storage
                // immediately so it survives a crash/kill, and again on
                // autosave/exit via `App::save`.
                if ui
                    .checkbox(&mut self.dark_mode, "Dark")
                    .on_hover_text("Invert pages (black background) and use the dark theme")
                    .changed()
                {
                    self.apply_theme(ctx);
                    if let Some(storage) = frame.storage_mut() {
                        storage.set_string(KEY_DARK_MODE, self.dark_mode.to_string());
                        storage.flush();
                    }
                }
                ui.checkbox(&mut self.show_debug, "Debug")
                    .on_hover_text("Show the frame-time / RSS / cache overlay");
                // Fase 5 chat toggle: opens the AI chat panel. Best-effort —
                // it asks about the visible page's text, so it is disabled
                // without an open document (same pattern as the Draw toggle).
                if has_doc {
                    ui.toggle_value(&mut self.chat_open, "💬 Chat")
                } else {
                    ui.add_enabled(false, egui::Button::selectable(false, "💬 Chat"))
                }
                .on_hover_text(
                    "Chat IA: pregunta sobre la página visible (requiere Ollama en localhost:11434)",
                );
                // Fase 3.5 annotations panel toggle: opens the side panel
                // with every annotation of the document (page + type +
                // summary); clicking an entry jumps to its page.
                if has_doc {
                    ui.toggle_value(
                        &mut self.annot_panel_open,
                        format!("Anotaciones ({})", self.annotations.len()),
                    )
                } else {
                    ui.add_enabled(false, egui::Button::selectable(false, "Anotaciones"))
                }
                .on_hover_text("Show all annotations; click one to jump to its page");
                // Fase 3-4 closure: persistence, export, clear. Exports run
                // on a background thread (`start_export`) and are disabled
                // while one is in flight; the result appears in the status
                // line. Clear empties the set and writes the empty sidecar.
                let export_busy = self.export_rx.is_some();
                if ui
                    .add_enabled(has_doc && !export_busy, egui::Button::new("Export MD"))
                    .on_hover_text("Exportar las anotaciones a Markdown (<pdf>.md, junto al PDF)")
                    .clicked()
                {
                    self.start_export(ExportKind::Markdown);
                }
                if ui
                    .add_enabled(has_doc && !export_busy, egui::Button::new("Export PDF"))
                    .on_hover_text("Exportar un PDF anotado (<pdf>.annotated.pdf, junto al PDF)")
                    .clicked()
                {
                    self.start_export(ExportKind::Pdf);
                }
                if ui
                    .add_enabled(has_doc, egui::Button::new("Clear"))
                    .on_hover_text("Borrar todas las anotaciones y guardar el sidecar vacío")
                    .clicked()
                {
                    // A fresh set is the same as deleting every stroke (ids
                    // restart from 0 — safe, no rows remain in the sidecar)
                    // and the empty sidecar is written immediately.
                    self.annotations = AnnotationSet::new();
                    self.save_annotations();
                }
            });
        });

        // Fase 3.5 annotations panel: a right side panel listing every
        // annotation of the document (page + type + summary). Clicking an
        // entry sets `pending_jump`, which `scroll_body` consumes next frame
        // to scroll-and-center that page (`ui.scroll_to_rect`). Shown only
        // with an open document; the toggle lives in the toolbar. Iterating
        // `0..page_count` with `for_page` is O(pages) HashMap lookups — cheap
        // (and the panel is closed by default), so no index is kept in sync.
        if self.annot_panel_open && self.prefetcher.is_some() {
            egui::SidePanel::right("annotations-panel")
                .resizable(true)
                .default_width(300.0)
                .show(ctx, |ui| {
                    ui.heading(format!("Anotaciones ({})", self.annotations.len()));
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.annotations.is_empty() {
                                ui.label(egui::RichText::new("Sin anotaciones todavía.").weak());
                            }
                            // Snapshot the rows first (page, id, label, is a
                            // text note): owned data, so the render loop below
                            // can mutate `self` (delete / open the edit input)
                            // without fighting the `for_page` borrow. Same
                            // order as before — page ascending, insertion
                            // order within a page.
                            let mut entries: Vec<(usize, u64, String, bool)> = Vec::new();
                            for page in 0..self.page_count as usize {
                                for ann in self.annotations.for_page(page) {
                                    let label = format!(
                                        "Page {} — {}",
                                        page + 1,
                                        annotation_summary(&ann.kind)
                                    );
                                    entries.push((
                                        page,
                                        ann.id,
                                        label,
                                        matches!(ann.kind, Annotation::TextNote(_)),
                                    ));
                                }
                            }
                            for (page, id, label, is_note) in entries {
                                // Right-to-left so the actions sit at the row's
                                // right edge while the selectable label fills
                                // the remaining width (truncating long
                                // summaries on a narrow panel). The ✕/✎ are
                                // separate widgets, so clicking them never
                                // triggers the row's jump-to-page.
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let del =
                                            ui.small_button("✕").on_hover_text("Delete annotation");
                                        if del.clicked() {
                                            // Delete from the set and persist
                                            // right away (same as every other
                                            // mutation); the overlay repaints
                                            // on this frame's CentralPanel.
                                            self.annotations.remove(id);
                                            if self.annot_selected == Some(id) {
                                                self.annot_selected = None;
                                            }
                                            self.save_annotations();
                                        }
                                        if is_note {
                                            let edit = ui
                                                .small_button("✎")
                                                .on_hover_text("Edit note text");
                                            if edit.clicked() {
                                                // Re-read the note (anchor +
                                                // current text) with a short
                                                // immutable borrow, then open
                                                // the same floating input used
                                                // at creation, pre-filled.
                                                let pos = edit.rect.min;
                                                let data = self
                                                    .annotations
                                                    .for_page(page)
                                                    .into_iter()
                                                    .find(|a| a.id == id)
                                                    .and_then(|a| match &a.kind {
                                                        Annotation::TextNote(n) => {
                                                            Some((n.anchor, n.text.clone()))
                                                        }
                                                        _ => None,
                                                    });
                                                if let Some((anchor, text)) = data {
                                                    self.open_note_input(
                                                        page,
                                                        anchor,
                                                        pos,
                                                        text,
                                                        Some(id),
                                                    );
                                                }
                                            }
                                        }
                                        // The row itself: a normal click jumps
                                        // to the annotation's page and marks
                                        // it as selected.
                                        let row = ui.add(
                                            egui::Button::selectable(
                                                self.annot_selected == Some(id),
                                                label,
                                            )
                                            .truncate(),
                                        );
                                        if row.clicked() {
                                            self.pending_jump = Some(page);
                                            self.annot_selected = Some(id);
                                        }
                                    },
                                );
                            }
                        });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            // Zoom input (ctrl+wheel, trackpad pinch). egui routes ctrl+wheel
            // to `zoom_delta` and away from `smooth_scroll_delta`, so the
            // ScrollArea below does not also pan while zooming.
            let zoom_delta = ui.input(|i| i.zoom_delta());
            if zoom_delta != 1.0 {
                self.set_zoom(self.zoom * zoom_delta);
            }

            // Ladder level for the current zoom. `scale_level_for_zoom` maps a
            // continuous zoom to the smallest 2^level ≥ zoom; multiplying by
            // the device pixel ratio keeps the old single-page path's "render
            // at screen resolution, never above" rule (AGENTS.md §4.4): on a
            // 2× display, 100% zoom renders at level 1 (144 dpi) and the GPU
            // downscales into the `page_size × zoom` logical rect.
            let level = scale_level_for_zoom(self.zoom * ctx.pixels_per_point());

            // Zoom crossed a ladder boundary (or first frame after open):
            // old-level textures are dead weight — each level is a distinct
            // render, so they can never be reused. Drop them (frees GPU
            // memory), drop in-flight receivers (their replies would be
            // stale-level bitmaps) and force a fresh request at `level`.
            if self.request_level != Some(level) {
                self.textures.clear();
                self.pending.clear();
                self.last_get.clear();
                self.last_request = None;
                self.last_request_at = None;
                self.request_level = Some(level);
            }

            // Take the prefetcher out of `self` for the frame so the scroll
            // closure can mutate the UI state while holding it (borrow
            // checker); it is put back before the frame ends.
            let prefetcher = self.prefetcher.take();
            match prefetcher.as_ref() {
                None => {
                    ui.label("Open a PDF to begin (or pass a path as argument).");
                    // Fase 3.5: the empty state also offers the recently
                    // opened PDFs (same list as the Open menu), so a fresh
                    // start is one click away.
                    if !self.recent_pdfs.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("Recents:").strong());
                        // Clone: `self.open` needs `&mut self`, which would
                        // clash with borrowing the list during iteration.
                        for path in self.recent_pdfs.clone() {
                            if ui.button(path.display().to_string()).clicked() {
                                match self.open(path.clone()) {
                                    Ok(()) => self.push_recent(path),
                                    Err(e) => self.status = format!("error opening: {e}"),
                                }
                            }
                        }
                    }
                }
                Some(p) => {
                    let total = self.page_count as usize;
                    // Salt the ScrollArea id with the open counter: egui's
                    // scroll state is keyed by id, so a fresh id forgets the
                    // previous document's position (open starts at the top).
                    let scroll_id = self.open_counter;
                    let mut scroll = egui::ScrollArea::vertical()
                        .id_salt(("pdf-scroll", scroll_id))
                        .auto_shrink([false, false]);
                    // Fase 3/3.5: while a tool is active a drag must draw /
                    // highlight / a click must note — not scroll — so disable
                    // the ScrollArea's content-drag; it never fights the
                    // capture (wheel/trackpad/scrollbar still scroll;
                    // `ScrollSource::drag` only gates the content-drag
                    // sensing).
                    if self.tool != ToolMode::Scroll {
                        scroll =
                            scroll.scroll_source(egui::containers::scroll_area::ScrollSource {
                                drag: false,
                                ..Default::default()
                            });
                    }
                    scroll.show_viewport(ui, |ui, viewport| {
                        self.scroll_body(ui, p, level, total, viewport);
                    });
                }
            }
            self.prefetcher = prefetcher;
        });

        // Fase 3.5 note input: a small floating area at the click position
        // (screen space; the *anchor* is fixed in page coordinates at click
        // time, so the note stays glued to the page across zoom/scroll). The
        // TextEdit is focused on the first frame so the user can type
        // immediately. Enter or a click elsewhere commits (`commit_note`),
        // Escape cancels — a stray click never discards typed text.
        if let Some(note) = &mut self.note_input {
            let mut commit = false;
            let mut cancel = false;
            egui::Area::new(egui::Id::new("note-input"))
                .fixed_pos(note.pos + egui::vec2(10.0, 10.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(
                            egui::RichText::new("Note — Enter: save · Esc: cancel")
                                .weak()
                                .small(),
                        );
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut note.text)
                                .desired_width(220.0)
                                .hint_text("Note text…"),
                        );
                        if !note.focus_requested {
                            note.focus_requested = true;
                            edit.request_focus();
                        }
                        commit = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                        // A click outside the input commits (the text survives
                        // a stray click; Esc is the explicit cancel). The
                        // grace period keeps the input-opening click (whose
                        // release may arrive at the same spot, outside the
                        // area) from committing an empty note.
                        let clicked_outside = ui.input(|i| i.pointer.any_click())
                            && note.opened_at.elapsed() > Duration::from_millis(200)
                            && !ui.rect_contains_pointer(ui.max_rect());
                        if clicked_outside {
                            commit = true;
                        }
                    });
                });
            if commit {
                self.commit_note();
            }
            if cancel {
                self.note_input = None;
            }
        }

        // Fase 5 chat panel: a floating Window (like the debug overlay), so
        // toggling it never reflows the scroll viewport. All work happens on
        // the background thread; here we only render the history and the
        // one-line input. `open` is a local copy (egui writes the close state
        // into it when the X is clicked) so the closure below can still use
        // `&mut self` freely; the result is synced back after `show`.
        if self.chat_open {
            let mut open = self.chat_open;
            let screen = ctx.screen_rect();
            egui::Window::new("Chat")
                .open(&mut open)
                .default_pos(egui::pos2(screen.right() - 400.0, 80.0))
                .default_width(380.0)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let edit = ui.add(
                            egui::TextEdit::singleline(&mut self.chat_input)
                                .hint_text("Pregunta sobre la página visible…")
                                .desired_width(f32::INFINITY),
                        );
                        let enter =
                            edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        let ask = ui
                            .add_enabled(!self.chat_busy, egui::Button::new("Ask"))
                            .clicked();
                        if (ask || enter) && !self.chat_busy && !self.chat_input.trim().is_empty()
                        {
                            self.submit_chat();
                        }
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.chat_history.is_empty() {
                                ui.label(
                                    egui::RichText::new(
                                        "Pregunta algo sobre la página visible; la respuesta usa su texto como contexto.",
                                    )
                                    .weak(),
                                );
                            }
                            for entry in &self.chat_history {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Tú (pág. {}): {}",
                                        entry.page + 1,
                                        entry.question
                                    ))
                                    .strong(),
                                );
                                match &entry.answer {
                                    Some(Ok(text)) => {
                                        ui.label(format!("Asistente: {text}"));
                                    }
                                    Some(Err(msg)) => {
                                        ui.colored_label(
                                            egui::Color32::RED,
                                            format!("Asistente: {msg}"),
                                        );
                                    }
                                    // In flight: the background thread is
                                    // talking to Ollama.
                                    None => {
                                        ui.label(egui::RichText::new("Asistente: …").italics());
                                    }
                                }
                                ui.separator();
                            }
                        });
                });
            self.chat_open = open;
        }

        // Debug overlay (Fase 1): frame time (current + p95), RSS and cache
        // state, refreshed every frame while visible. A floating Window is
        // independent of the panels, so it stays readable while scrolling.
        if self.show_debug {
            let stats = self
                .prefetcher
                .as_ref()
                .map(|p| p.stats_snapshot())
                .unwrap_or_default();
            egui::Window::new("Debug")
                .default_pos([20.0, 80.0])
                .show(ctx, |ui| {
                    // Reference target, so the author sees at a glance
                    // whether the scroll meets the 60 fps budget (AGENTS.md §8).
                    ui.label(egui::RichText::new("Target: p95 < 16.6 ms (60 fps)").strong());
                    ui.separator();
                    ui.label(format!(
                        "frame: {:.2} ms",
                        self.last_frame.as_secs_f64() * 1000.0
                    ));
                    ui.label(format!(
                        "p95: {:.2} ms",
                        self.frame_timer
                            .p95()
                            .unwrap_or(Duration::ZERO)
                            .as_secs_f64()
                            * 1000.0
                    ));
                    match read_rss_kb() {
                        Some(kb) => ui.label(format!("RSS: {:.1} MB", kb as f64 / 1024.0)),
                        None => ui.label("RSS: n/a (/proc unavailable)"),
                    };
                    ui.separator();
                    ui.label(format!("cache hits: {}", stats.hits));
                    ui.label(format!("cache misses: {}", stats.misses));
                    ui.label(format!("cache evictions: {}", stats.evictions));
                    ui.label(format!(
                        "cache bytes: {:.1} MB",
                        stats.current_bytes as f64 / (1024.0 * 1024.0)
                    ));
                    ui.label(format!("cache entries: {}", stats.entries));
                });
        }
    }

    /// Persists the preferences on eframe's autosave/exit: the dark-mode
    /// preference (also written immediately on toggle, see `update`) and the
    /// recents list (also flushed right after `push_recent`, see `update`).
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        storage.set_string(KEY_DARK_MODE, self.dark_mode.to_string());
        eframe::set_value(storage, KEY_RECENTS, &self.recent_pdfs);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(text: &str, x: f32, y: f32, w: f32, h: f32) -> TextSpan {
        TextSpan {
            text: text.to_string(),
            x,
            y,
            w,
            h,
        }
    }

    /// Two full lines (10 px high) plus a partial third, in a 600 px wide page.
    fn sample_spans() -> Vec<TextSpan> {
        vec![
            span("line one", 50.0, 10.0, 300.0, 10.0),
            span("line two", 50.0, 22.0, 280.0, 10.0),
            span("partial", 50.0, 34.0, 120.0, 10.0),
        ]
    }

    #[test]
    fn drag_clips_spans_horizontally_and_keeps_full_line_height() {
        // Drag from x=100 to x=200 over the first two lines: each covered line
        // yields a rect clipped to the drag's x-range, with the full line
        // height (the highlighter covers whole line boxes). The drag's
        // y-range (5..40) also overlaps the third line's bbox (34..44), so it
        // is selected too — any overlap with a line's bbox selects the line.
        let drag = Rect::new(100.0, 5.0, 100.0, 35.0);
        let rects = highlight_rects_for_drag(&sample_spans(), drag);
        assert_eq!(rects.len(), 3);
        assert_eq!(rects[0], Rect::new(100.0, 10.0, 100.0, 10.0));
        assert_eq!(rects[1], Rect::new(100.0, 22.0, 100.0, 10.0));
        // The partial line ends at x=170: the drag to x=200 stops there.
        assert_eq!(rects[2], Rect::new(100.0, 34.0, 70.0, 10.0));
    }

    #[test]
    fn drag_over_partial_line_clips_to_line_end() {
        // Drag from x=100 to x=200 starting inside line two: both line two
        // (overlapped from y=30) and the partial line are selected; the
        // partial line's rect stops at its own end (x=170).
        let drag = Rect::new(100.0, 30.0, 100.0, 20.0);
        let rects = highlight_rects_for_drag(&sample_spans(), drag);
        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0], Rect::new(100.0, 22.0, 100.0, 10.0));
        assert_eq!(rects[1], Rect::new(100.0, 34.0, 70.0, 10.0));
    }

    #[test]
    fn drag_over_empty_space_yields_no_rects() {
        // No span intersects the margin below the text: the caller falls back
        // to the single drag rect (documented behaviour).
        let drag = Rect::new(400.0, 400.0, 100.0, 50.0);
        assert!(highlight_rects_for_drag(&sample_spans(), drag).is_empty());
    }

    #[test]
    fn drag_in_the_gap_between_lines_selects_nothing() {
        // The gap between line one (ends y=20) and line two (starts y=22): a
        // drag entirely inside it overlaps no line bbox → no rects (brushing
        // a line edge does not select it).
        let drag = Rect::new(50.0, 20.5, 200.0, 1.0);
        assert!(highlight_rects_for_drag(&sample_spans(), drag).is_empty());
    }
}
