# Spec de diseño — Reestructuración integral de PDFLector

**Fecha:** 2026-09-06
**Estado:** Aprobado por el usuario (secciones 1-4, sesión del 2026-09-06)
**Baseline:** `main` @ 2247069. El diff sin commit de `gpu.rs` (+32/−22, worktree `code_reviuw`) se revierte como primer paso de la Fase 2.
**Idioma:** español en docs, inglés en código/commits.

## 0. Objetivo y enfoque

Plan de reestructuración INTEGRAL de PDFLector (código + documentación + gobernanza), punto por punto, con un criterio de verificación por punto. Enfoque aprobado: **Híbrido Progresivo** (C), ejecutado en 4 fases en este orden:

1. **Fase 1 — Docs & Gobernanza**: cerrar los 12 hallazgos de obsolescencia, reescribir activos, congelar históricos, índice maestro.
2. **Fase 2 — GPU**: revert del diff corrupto + reordenación del pipeline present (dry = fondo, overlays a fb0) + fix `ovl_cache` ABA + fix `EGL_BAD_ALLOC` Library→Viewer.
3. **Fase 3 — CI Android**: job de compilación cruzada en GitHub Actions.
4. **Fase 4 — Megaficheros**: extracción modular en 2 capas de reader/draw/gpu/input.

Fuera de alcance: bug de pantalla apagada (queda abierto en NEXT-PLAN), Fase D (IA), Sync, medición TCL de las fases A4/A5/E2/E3 pendientes (el plan solo añade las mediciones que le son propias: Fase 2).

Prioridades de producto vigentes (AGENTS.md): Fluidez (p95 <16.6ms, render <25ms TCL) > Memoria (PSS <150MB) > resto.

## 1. Fase 1 — Docs & Gobernanza

### 1.1 Decisiones de gobernanza

- **G1 — CHANGELOG completo**: `CHANGELOG.md` se mantiene entero, más reciente arriba. La regla "últimas 5" se corrige en `AGENTS.md:70` y en la cabecera de `CHANGELOG.md:3` (hoy hay 19 entradas, 2026-08-13 → 2026-09-05).
- **G2 — ADRs = snapshots inmutables**: los ADR no se reescriben (cifras antiguas de ADR-005:16, refs rotas de ADR-003:22-23,176 quedan como están). Solo se normaliza el **estado** de ADR-007: "Aceptado (propuesto)" → "Aceptado", aclarando que supersede **parcialmente** el present de ADR-006 (que sigue vigente para el resto). La convención queda escrita en el índice maestro.
- **G3 — Índice maestro**: crear `docs/README.md` con toda la documentación y su estado (activo/histórico/superseded) + la convención G2. Test de completitud: el conjunto de ficheros de `find docs -name '*.md'` menos los listados en el índice debe ser vacío.

### 1.2 Ficheros activos que se reescriben

| Fichero | Cambios (anclajes verificados) |
|---|---|
| `AGENTS.md` | :34 quitar el límite-50 como "tarea futura en Fase E" (E4 ya lo ejecutó; política "nunca borra" queda). :70 regla CHANGELOG → completo. |
| `docs/PROYECTO.md` | :125 quitar borrado-auto como pendiente. :27 "paginado" se mantiene (correcto: una hoja + tap desde 2026-08-13, CHANGELOG:285-288). |
| `docs/plan/NEXT-PLAN.md` | :6 presupuesto PSS con mediciones reales (52.9MB arranque / 105MB lectura / 208MB tras 130 page-turns, CHANGELOG:36-46,28-34 + A-latencia:21). :34-38 estado auditado desfasado → reflejar A1-A3 [x], B cerrada, C1-C4 [x], E1/E4 [x]. :37 cifras de líneas (reader.rs 5605, draw.rs 4899). **Nueva sección "Deuda transversal"**: EGL_BAD_ALLOC Library→Viewer (se cierra en Fase 2 de este plan), verificación ADR-007 §8.4 pendiente (se cierra en Fase 2), bug pantalla apagada (abierto, medición TCL), 419 unwraps en tests/benches. |
| `docs/plan/00-objetivo.md` | :21 "scroll continuo" → paginado una-hoja (doble error hoy: dice "descartado el paginado, autor prefiere scroll continuo" cuando el producto es paginado). :30 fases waterfall descartadas → referencia a NEXT-PLAN como único roadmap. :31-32 Obsidian/Syncthing → marcar fuera de v1 (NEXT-PLAN:4,24). :20 "decisión 2" inexistente → la real vive en PROYECTO:127-128. |
| `docs/plan/README.md` | :8-14 criterio de issue armonizado con NEXT-PLAN:28. Apuntar a `docs/README.md` como índice maestro. |
| `docs/plan/COMPETENCIA.md` | :9 "composite 200 trazos: no medido aún" → ya medido: 4.39ms directo / 2.39ms StrokeCache (benchmark-results:561-575). |
| `docs/plan/D-ia-contexto.md` | :21 ref rota `tools/ai-bench.sh` → marcar "por crear en D4". |
| `CHANGELOG.md` | Entrada de la reestructuración + cabecera según G1. |
| `README.md` | Sección build Android (comandos NDK r28 de AGENTS.md) + enlaces a docs/adr, benchmark-results y skills. |
| `CONTRIBUTING.md` | Bloque de verificación local (test pdf_core, clippy, check Android). |
| `.github/PULL_REQUEST_TEMPLATE.md` | Ítem "check Android local ejecutado" (parapeto hasta Fase 3). |

### 1.3 Históricos que se congelan (banner, contenido intacto)

Ya congelados: `docs/PLAN.md`, `docs/PLAN-UX-UI-NUEVO.md` (desde 2026-09-04), `docs/plan/PLAN-IMPLEMENTACION-TECNICA-PRO.md`, `docs/plan/PLAN-UX-UI-PRO.md` (con banner pero fuera de índice → G3 los indexa), `docs/api-anotaciones-ui.md` (superseded), `docs/adr/ADR-004` (superseded), `memory.md` (redirect intencional).

Faltan banner: `docs/auditoria-contexto-agentes-2026-08-23.md` (nota: skills fantasma :23,102-105,155-156 no existen), `docs/ux-rediseño-estructura.md`, `docs/api-anotaciones-fase3.md`, `docs/plan/01-06`, `docs/research/*` (10), `docs/investigacion/*`, `docs/benchmarks/*`. Excepción: `docs/legal.md` NO se congela (vigente).

### 1.4 Cierre de los 12 hallazgos

1→banner §1.3 · 2→G1 · 3→PROYECTO §1.2 · 4→COMPETENCIA §1.2 · 5→D-ia:21 corregido + ADR-003/shootout por G2+banner · 6→NEXT-PLAN:37 corregido + ADR-005 por G2 · 7→00-objetivo §1.2 · 8→banner ya puesto, índice por G3 · 9→G3 · 10→deuda transversal NEXT-PLAN, verificación en Fase 2 · 11→estado ADR-007 + G3 · 12→G3 (redirect intencional documentado).

### 1.5 Verificación (criterio por punto)

- V1.1: `grep -rn "últimas 5" AGENTS.md CHANGELOG.md` → 0 hits.
- V1.2: `grep -n "scroll continuo" docs/plan/00-objetivo.md` → 0 hits.
- V1.3: `grep -n "no medido aún" docs/plan/COMPETENCIA.md` → 0 hits.
- V1.4: `grep -n "tarea futura" AGENTS.md` (en contexto límite-50/E) → 0 hits.
- V1.5: refs rotas en docs activos → 0 (re-escaneo del patrón del scout: rutas `tools/`, `docs/`, `crates/` citadas que no existen; históricos congelados excluidos).
- V1.6: `find docs -name '*.md'` vs listado en `docs/README.md` → diferencia vacía.
- V1.7: cada checkbox [x] de fases A-E cita evidencia con fecha+hardware+flujo+métrica (regla AGENTS.md).
- V1.8: comandos de verificación local citados en README/CONTRIBUTING existen y corren (dry run).

## 2. Fase 2 — GPU (revert + pipeline limpio + EGL_BAD_ALLOC)

### 2.1 Estado verificado (contra `main` @ 2247069)

- `present_viewer` gpu.rs:1740; DryKey :485-495 = `{page, zoom_bits, pan_x, pan_y, ann_count, dark, chrome_visible, sheet_progress_bits, has_toast}` → cada pan/chrome/sheet/toast invalida la dry completa.
- `render_dry` :1421 pinta página + anotaciones + sheet (:1554-1560) + chrome + toast dentro de la dry.
- `ovl_cache` :947-997: clave `b.data.as_ptr()` + `_keep: b.data.clone()` (:989) → **bug ABA** (dirección reutilizada por el allocator devuelve textura con contenido equivocado); límite 8 entradas por conteo (:992), sin presupuesto por bytes.
- `main` está SANO: guard `has_wet` :1767-1770, swap interval dinámico :1786-1790, `draw_fullscreen_texture` :1341 llamada con 2 args. Los 9 hallazgos críticos de la auditoría describían el worktree `code_reviuw` CON el diff aplicado (no compila: `}` suelto :1832, doble draw, wet sin guard en el diff).

### 2.2 Diseño

**A. Reordenar el pipeline:**

```
present_viewer:
  1. dry_fbo ← página + anotaciones SOLO (sin UI)           [render_dry]
  2. overlays (chrome, sheet, toast, sel_menu, ai_panel,
     lib_fade, badges) → texturas via ovl_cache             [draw_bitmap directos]
  3. fb0 ← dry (pan/zoom en el quad) ⊕ wet (si has_wet) ⊕ overlays
```

- `DryKey` se reduce a `{page, zoom_bits, ann_count, dark}` (9→4 campos): pan, chrome, sheet y toast dejan de invalidar la dry.
- Ganancias: pan fluido sin re-raster del contenido; `sheet_progress` deja de forzar ~100 re-renders por animación; `lib_fade` se anima sin invalidar.
- `render_dry` pierde las secciones de UI (:1554-1578 aprox) que pasan al pase de overlays.
- `draw_fullscreen_texture` :1341 gana offset `(dx, dy)` para el translate del pan (lo que el diff corrupto pretendía, implementado honestamente).
- `set_swap_interval` :1259 se mantiene basado en gesto activo (ya correcto en main).

**B. Correcciones:**
- `ovl_cache` ABA :947-997: clave pasa de `as_ptr()` a id estable del bitmap del llamador (u64 monotónico en el propietario del bitmap, o hash ligero contenido+len) + presupuesto **por bytes** (regla repo: LRU por bytes como `cache.rs`/`thumbs.rs`).
- Cursor de goma: `has_erase_pt` sale de DryKey; el cursor vive en el pase de overlays con posición viva (:1899-1907 se elimina; el círculo vivo en wet :1719-1731 se mantiene).
- `has_selection` sale de DryKey (:494-495): el rect se dibuja en la wet (:1734-1737), no en la dry.

**C. EGL_BAD_ALLOC Library→Viewer (0x3003):**
Hipótesis: al volver de Library (blits SW, surface dropeada) a Viewer, `recreate_surface` :691-725 / `make_resources` re-crean recursos sin liberar versiones previas en alguna ruta (`drop_surface_only` :747 destruye los FBO; verificar re-creación). Plan: log de ciclo de vida (surface create/drop, FBO create/destroy con contadores) + fix. Criterio: 10 ciclos Library→Viewer consecutivos sin `EGL_BAD_ALLOC` en logcat, PSS estable.

### 2.3 Verificación

| Punto | Test |
|---|---|
| Revert diff | `git diff` vacío en code_reviuw + `cargo check -p pdf_android --target aarch64-linux-android` verde |
| DryKey reducida | Test unitario host (función pura extraída): cambiar pan/chrome/sheet/toast NO invalida; cambiar page/zoom/ann_count/dark SÍ |
| Overlays fuera de dry | Test unitario: animación completa de sheet NO re-invoca `render_dry` (contador de invocaciones) |
| ovl_cache sin ABA | Test unitario: bitmap A (cacheado) → drop → bitmap B reutiliza dirección → hit NO devuelve textura de A. Presupuesto: N bitmaps grandes → bytes residentes ≤ presupuesto |
| Wet guard | Test unitario: `present_viewer` sin gesto no re-renderiza la wet |
| EGL ciclo | Medición TCL: 10× Library→Viewer sin 0x3003 (logcat), PSS estable (dumpsys); entrada en benchmark-results.md con fecha+hardware+flujo+métrica |
| Rendimiento present | Medición TCL: p95 gl_present con pan activo antes/después (objetivo: pan sin re-raster, p95 <16.6ms sostenido); entrada en benchmark-results.md |

## 3. Fase 3 — CI Android

CI actual (.github/workflows/ci.yml): fmt + clippy + test pdf_core, host-only (`default-members` excluye pdf_android → el diff corrupto NO se habría detectado).

Diseño: job `android` paralelo (no bloquea el carril host) en ubuntu-latest:
1. checkout + toolchain stable con target `aarch64-linux-android`.
2. NDK r28 (setup cacheado; p.ej. `android-actions/setup-android-ndk` o descompresión manual con cache).
3. `cargo check -p pdf_android --target aarch64-linux-android` con `PATH` del NDK + `BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android=--sysroot=$NDK/toolchains/llvm/prebuilt/linux-x86_64/sysroot` (comandos de AGENTS.md ya validados localmente).

Actualización de gobernanza: AGENTS.md tabla de validación refleja el nuevo carril ("Android check en CI" pasa de "no en CI actual" a cubierto). README/CONTRIBUTING apuntan al carril.

### Verificación
- V3.1: workflow run verde en GitHub Actions con el job `android` en verde y tiempo reportado.
- V3.2: prueba de valor: aplicar un diff que rompa la compilación Android (p.ej. un `}` suelto) en una rama de prueba → el job `android` falla (rotura detectada).

## 4. Fase 4 — Megaficheros

### 4.1 Diseño (2 capas)

**Capa 1 — extracción mecánica de seams** (movimiento puro, sin tocar el struct `Reader`), usando los rangos por responsabilidad del scout EstructuraAndroid-2 (líneas contra `main`):
- `draw/` → `primitives.rs` (blits 35-107, copy_region 153), `tinta.rs` (272-437), `chrome.rs` (980-1272), `sheet.rs` (1273-1554), `menus.rs` (1555-2763), `library.rs` (2764-3545), `overlays.rs` (4487-4899).
- `gpu/` → `ffi.rs` (34-237), `shaders.rs` (267-378), `surface.rs` (444-946), `textures.rs` (947-1240), `pipeline.rs` (dry 1421-1575, wet 1576-1739, present 1740-1823).
- `input/` → `gestos.rs` (87-183), `motion.rs` (454-1350), `stylus.rs` (1624-1701), `dispatch.rs` (1537-1623).
- `reader/` → bloques `impl Reader`: `geometry.rs` (308-480), `life.rs` (1536-1830), `redraw.rs` (1833-2640), `seleccion.rs` (2495-2640), `sheet.rs` (3172-3474), `tick.rs` (3475-3600), `anotaciones.rs` (3980-4428), `tools.rs` (4457-4565), `navigation.rs` (4980-5235), `library.rs` (5315-5568).

**Capa 2 — extracción estructural `LibraryState`**: los ~30 campos `lib_*` del struct `Reader` (:1051-1443) salen a un struct `LibraryState` propio, poseído por `Reader` (`reader.library: LibraryState`), con sus métodos. `Reader` queda enfocado en el visor.

**Depuración transversal**: los 47 `#[allow(dead_code)]` inventariados se revisan durante la extracción: los que queden huérfanos se eliminan (post-extracción, el grep debe reducirse).

### 4.2 Verificación (por extracción)

- V4.1: `cargo check -p pdf_android --target aarch64-linux-android` verde tras cada módulo movido.
- V4.2: `cargo test -p pdf_core` verde tras cada módulo (sin regresión de lógica compartida).
- V4.3: `cargo clippy --all-targets -- -D warnings` verde (raíz default-members).
- V4.4: movimiento puro: `git diff` de cada extracción sin cambio de firmas `pub(crate)` ni de comportamiento.
- V4.5: objetivo de tamaño: cada fichero resultante <2000 líneas.
- V4.6: conteo de `#[allow(dead_code)]` estrictamente menor al final de la fase (baseline: 47).

## 5. Orden de ejecución y dependencias

```
Fase 1 (docs) ──► Fase 2 (gpu) ──► Fase 3 (CI) ──► Fase 4 (megaficheros)
```

- Fase 1 primero fija el marco de gobernanza (AGENTS.md/NEXT-PLAN) que las demás citan.
- Fase 2 deja el código Android compilable y medible; su medición TCL cierra también la deuda de ADR-007 §8.4 anotada en Fase 1.
- Fase 3 blinda antes de la Fase 4 (movimiento masivo de módulos guardado por el job Android).
- Fase 4 al final, sobre código verde y con CI como red de seguridad.

Restricciones globales de todo el plan: AGENTS.md MUST/MUST NOT vigente en todo momento (pdf_core sin UI, sin unwrap en producción, sin deps GPL, sin claves en git, biblioteca nunca borra PDFs); cada fase termina con commit atómico y CHANGELOG actualizado.
