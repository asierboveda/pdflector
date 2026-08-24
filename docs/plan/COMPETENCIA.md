# Competencia — Lectores PDF actuales (estudio rápido 2026-08-24)

> Para comparar tu visor. Medido en literatura + tu baseline TCL.

## Tu baseline TCL (2026-08-12, pdf_bench release, pantalla ON)

- `render1x`: dense 14.5ms, paper 11.6ms, large 15.4ms, scanned 31ms (worst)
- `PSS`: 26.7MB (harness) / 66MB (pdf_android release, visor) — objetivo <150MB con margen
- `composite_annotations` 200 trazos: no medido aún (Fase A lo medirá)
- Conclusión: MuPDF en TCL es competitivo, la latencia está en overlay/selección, no en render.

## Lectores de referencia

| Lector | Motor | Render 1x (estimado) | Anotaciones | IA | Notas para tu plan |
|--------|-------|----------------------|-------------|----|--------------------|
| **Xodo** (Android, líder) | PDFium | ~12ms | Subrayado con snapping a palabra, 0-latencia | Chat con contexto (Xodo AI) | Pre-extrae texto al abrir, R-tree para hit-test. Tu B1/B2 copia esto. |
| **Adobe Acrobat Reader** | Propio | ~18ms | Subrayado + lápiz con palm rejection | AI Assistant (contexto largo) | Usa display list + tiled render. Tu `fz_display_list` es similar. |
| **MuPDF Viewer** (Artifex) | MuPDF | ~10ms | Ink/Highlight básicos | No | Tu motor. Usa `fz_store` + `fz_cookie` para abortar render al scrollear. Copia el patrón. |
| **prime-pdf-viewer** (Rust+Slint) | MuPDF | ~11ms | No | No | **Tu gemelo.** Repo ByteApps, 0★, Rust+Slint, offline read-only. Prueba que tu stack funciona. |
| **KOReader** (e-ink) | MuPDF | ~15ms (e-ink) | Highlight con dict | No | Cache LRU por bytes desde RAM libre + persistencia a disco. Tu `cache.rs` es similar pero fijo 48MiB. |
| **Evince** (GNOME) | Poppler | ~29ms (poppler) | No | No | Ya auditado en ADR-002. Sliding window 50MB. Tu `prefetch.rs` es mejor (actor). |

## Qué copiar y qué no

- **Copiar:** pre-extracción de texto + índice espacial (Xodo), `StrokeCache` (KOReader), harness `adb` con `dumpsys` (todos), BM25 sin embeddings para IA (ChatPDF).
- **No copiar:** tiling completo (Okular) — innecesario para visor 1-página cover; WebView (Tauri) — rompe PSS 150MB; presión del lápiz — ya descartada.

## Tu ventaja

Ninguno combina: **Rust + MuPDF estático + offline + IA con visión + sidecar SQLite sync-friendly**. Ese es tu portfolio.

## Fuentes

- `ArtifexSoftware/mupdf-android-viewer` (184★), `ArtifexSoftware/mupdf-android-viewer-mini`
- `ByteApps/prime-pdf-viewer` (Rust+Slint)
- `koreader/koreader` (28k★), `DImuthuUpe/AndroidPdfViewer` (8k★), `pwmt/zathura`
- `docs/benchmark-results.md` (tu tabla), `docs/research/rendering-cache.md`
