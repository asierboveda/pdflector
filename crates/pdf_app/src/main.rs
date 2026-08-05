//! pdf_app — egui desktop prototype (docs/PLAN.md: final Android UI decided in
//! Phase 6; this app never holds logic, it only asks pdf_core and paints).

use std::path::PathBuf;

use eframe::egui;
use pdf_core::engine::pdfium::{PdfiumDocument, PdfiumEngine};
use pdf_core::{Document, RenderEngine};

fn pdfium_lib_path() -> PathBuf {
    // Dev convenience: resolve the vendored lib relative to the workspace.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../vendor/pdfium/lib/libpdfium.so")
}

fn main() -> eframe::Result<()> {
    let initial_pdf = std::env::args().nth(1).map(PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1024.0, 768.0]),
        ..Default::default()
    };
    eframe::run_native(
        "PDFLector",
        options,
        Box::new(move |cc| Ok(Box::new(App::new(cc, initial_pdf)))),
    )
}

struct App {
    engine: PdfiumEngine,
    doc: Option<PdfiumDocument>,
    texture: Option<egui::TextureHandle>,
    status: String,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>, initial_pdf: Option<PathBuf>) -> Self {
        let engine = PdfiumEngine::new(&pdfium_lib_path()).expect("failed to bind libpdfium");
        let mut app = Self {
            engine,
            doc: None,
            texture: None,
            status: "no document".to_string(),
        };
        if let Some(path) = initial_pdf {
            app.open(&cc.egui_ctx, path);
        }
        app
    }

    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        match self.engine.open(&path) {
            Ok(doc) => {
                self.status = format!("{} — {} pages", path.display(), doc.page_count());
                // Phase 0: render page 1 once, at a fixed scale. Caching, zoom
                // and background rendering are Phase 1 scope.
                match doc.render_page(0, 2.0) {
                    Ok(bmp) => {
                        let image = egui::ColorImage::from_rgba_unmultiplied(
                            [bmp.width as usize, bmp.height as usize],
                            &bmp.data,
                        );
                        self.texture =
                            Some(ctx.load_texture("page-0", image, egui::TextureOptions::LINEAR));
                    }
                    Err(e) => self.status = format!("render error: {e}"),
                }
                self.doc = Some(doc);
            }
            Err(e) => self.status = format!("error opening {}: {e}", path.display()),
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open PDF…").clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .add_filter("PDF", &["pdf"])
                        .pick_file()
                {
                    self.open(ctx, path);
                }
                ui.label(&self.status);
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both().show(ui, |ui| {
                if let Some(texture) = &self.texture {
                    ui.image((texture.id(), texture.size_vec2()));
                } else {
                    ui.label("Open a PDF to begin (or pass a path as argument).");
                }
            });
        });
    }
}
