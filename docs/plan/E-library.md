# Fase E — Biblioteca fluida en cuadrícula

Catálogo local de documentos, generación asíncrona de portadas en segundo plano y transiciones inmediatas hacia el visor.

## Auditoría

- Arquitectura modular en `crates/pdf_android/`:
  - `reader/library_state.rs` y `reader/mod.rs`: gestión del catálogo, ordenación, filtrado y posición de scroll en píxeles. El carrusel histórico "Continue Reading" fue eliminado (commit `3b726a1`) en favor de una cuadrícula uniforme y directa.
  - `draw/library.rs`: composición en GPU de la cuadrícula de portadas con recorte y badges de estado.
  - `thumbs.rs` (:197-267): `ThumbWorker` como actor en segundo plano con hilo propio y canal MPSC no bloqueante. Las portadas se generan de forma progresiva sin bloquear el hilo principal.
  - Transición fluida lista→visor: `compose_library_snapshot` y `blit_lib_fade` permiten abrir el documento sin saltos visuales.

## Objetivo

Scroll en cuadrícula sin caídas de frames y carga diferida de portadas sin interferir con la apertura o lectura de PDFs.

## Tareas

- [x] E1. **Portadas desacopladas de la UI**: `ThumbWorker` ejecutado en hilo secundario con instancia dedicada de `MupdfEngine`. El hilo principal solo consulta el canal con `try_recv()` no bloqueante en cada `tick()`. Medido en hardware TCL: blits de 5.56–6.36 ms (p95 6.1 ms) con 0 ms de I/O síncrono en la interfaz.
- [x] E2. **Menú contextual y hojas sin re-render**: Animación de la hoja de menú con frame cacheado. Verificado el 2026-09-07 en TCL: apertura y cierre en 13 presents sin re-renderizar la página PDF ni desalojar cachés.
- [ ] E3. **Escala a catálogo de 256 libros**: Medir rendimiento de scroll en cuadrícula con 256 documentos. Medición parcial el 2026-09-07: p95 de 10.0 ms con 11 libros. La variante de 256 libros permanece pendiente.
- [x] E4. **Eliminación de borrado automático de libros**: Eliminación de `enforce_library_limit`, de la constante `LIBRARY_MAX` (antiguo tope de 50 libros) y de llamadas a `fs::remove_file`. La biblioteca nunca elimina ficheros del usuario automáticamente.

## Criterio de cierre

- [ ] Cuadrícula con 256 libros: scroll continuo con p95 < 16.6 ms en la tablet TCL (medido p95 10.0 ms con 11 libros; variante 256 libros `SIN MEDIR`).
- [ ] Primer frame interactivo del visor en cold-start < 200 ms (mediana actual medida: 349 ms; requiere optimización de apertura).

## Cómo modificar

- Para alternar entre cuadrícula y lista compacta, ajustar el cálculo de geometría en `crates/pdf_android/src/draw/library.rs`.
- El tamaño y formato de miniaturas se configura en `crates/pdf_android/src/thumbs.rs`.

## Referencias

- `crates/pdf_android/src/thumbs.rs`
- `crates/pdf_android/src/reader/library_state.rs`
- `crates/pdf_android/src/draw/library.rs`
- `docs/benchmark-results.md`
