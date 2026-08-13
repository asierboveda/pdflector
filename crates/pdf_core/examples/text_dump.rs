//! Smoke test de extracción de texto: imprime el texto plano y los spans (bbox)
//! de una página, para comprobar la calidad de la extracción MuPDF stext.
use pdf_core::engine::mupdf::MupdfEngine;
use pdf_core::{Document, RenderEngine};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("uso: text_dump <pdf> [pagina]");
    let page: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0);

    let engine = MupdfEngine::new().expect("mupdf");
    let doc = engine.open(std::path::Path::new(&path)).expect("open");
    let total = doc.page_count();
    let text = doc.text(page).expect("text");

    println!("=== {}  (página {}/{}) ===", path, page + 1, total);
    println!("--- TEXTO PLANO ---");
    println!("{}", text.text);
    println!("--- SPANS ({} totales, primeros 50) ---", text.spans.len());
    for (i, s) in text.spans.iter().take(50).enumerate() {
        println!(
            "[{i:3}] x={:6.1} y={:6.1} w={:6.1} h={:5.1}  {:?}",
            s.x, s.y, s.w, s.h, s.text
        );
    }
}
