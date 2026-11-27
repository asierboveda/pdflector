# PDFLector — Benchmark & Hardware Profiling Results

> Formato obligatorio (`pdflector-rendimiento` §7): **Fecha + Hardware + Flujo medido + Métrica**.  
> Este documento es *append-only*: cada campaña añade una nueva sección fechada sin borrar históricos. Cualquier flujo no medido explícitamente se marca `SIN MEDIR`.

---

## 2026-11-27 — Baseline `origin/main` (`fb2cc5c`)

### Hardware
- **Desktop (x86_64 Linux):** AMD Ryzen 7 5800H (16 threads), 13 GiB RAM, Rust release (`cargo bench` / `cargo run --release`).
- **Tablet (ARM64 Android):** TCL NXTPaper 11 Plus (`TCL 9469X`, serial `A06B4A8E6774623`), Android 16 (SDK 36), MediaTek Helio G100 (8 cores `arm64-v8a`), Mali-G57 MC2 (GLES2/EGL), 8 GB RAM, display `1440×2200` @ `120 Hz` (`mActiveModeId=1`, vsync `8.33 ms`).
- **Build:** `libpdf_android.so` release ARM64 (`16 MB`, NDK r28 API 26, sin `google_key.txt`), `app-debug.apk` (`25 MB`, `assembleProductDebug`).
- **Corpus aislado:** `dense_textbook.pdf` (93p, 82 KB), `large_document.pdf` (500p, 530 KB), `scanned_pages.pdf` (30p, 24 KB), `scientific_paper.pdf` (12p, 23 KB), `mixed_stress.pdf` (80p mixed portrait/landscape + vector/matrix/text, 567 KB).

---

### 1. Desktop — `pdf_bench` Sweep (x86_64, release)

| Flujo medido | Métrica |
|---|---|
| `dense_textbook.pdf` (93p) — open / render1x / render2x | `0.11 ms` / `2.78 ms` / `7.82 ms` |
| `scanned_pages.pdf` (30p) — open / render1x / render2x | `0.08 ms` / `8.45 ms` / `34.20 ms` |
| `scientific_paper.pdf` (12p) — open / render1x / render2x | `0.09 ms` / `1.88 ms` / `6.31 ms` |
| `large_document.pdf` (500p) — open / render1x / render2x | `0.13 ms` / `3.05 ms` / `8.53 ms` |
| `pdf_bench` sweep — `PEAK_RSS_KB` | `25,180 KB` (`24.6 MiB`) |
| `zoom large p0` — `scale2x` / `scale4x` / `rerender1` / `rerender2` | `50.56 ms` / `202.87 ms` / `2.65 ms` / `10.52 ms` |

### 2. Desktop — Criterion Suites (`cargo bench -p pdf_bench -- --quick`, median)

| Suite / Flujo medido | Métrica |
|---|---|
| `open_render` — open `dense` / `scanned` / `paper` / `large` | `35.4 µs` / `34.9 µs` / `33.3 µs` / `60.1 µs` |
| `open_render` — `render_1x` `dense` / `large` | `3.45 ms` / `5.28 ms` |
| `open_render` — `render_2x` `dense` / `large` | `5.40 ms` / `7.23 ms` |
| `cache_scroll` — `naive_hold_50p_1x` / `cache_8mb_firstpass` / `cache_8mb_pass2` | `109.48 ms` / `77.01 ms` / `989.8 µs` |
| `cache_scroll` — peak `VmHWM` (`naive` / `cache_8mb` / `cache_pass2`) | `109,636 KB` / `23,916 KB` / `23,856 KB` |
| `render_perf` — `paper_12p` `s1x` / `s2x` | `611.3 µs` / `2.10 ms` |
| `render_perf` — `scanned_30p` `s1x` / `s2x` | `2.87 ms` / `11.26 ms` |
| `render_perf` — `dense_93p` `s1x` / `s2x` | `1.17 ms` / `2.90 ms` |
| `render_perf` — `large_500p` `s1x` | `1.02 ms` |
| `zoom` — `scale_bitmap` `z2` / `z4` | `52.61 ms` / `207.38 ms` |
| `zoom` — `rerender` `level1_x2` / `level2_x4` / `trim_to_scale_level` | `2.72 ms` / `10.58 ms` / `3.62 ms` |
| `annotations` — `add` n100 / n1000; `for_page` (200 of 1000) | `6.20 µs` / `56.51 µs`; `67.7 ns` |
| `annotations` — JSON serialize/deserialize n100 / n1000 | `24.9 / 36.6 µs`; `246.1 / 386.7 µs` |
| `annotations` — SQLite `store_roundtrip` n100 / n1000 | `17.20 ms` / `36.75 ms` |
| `blit` — `page_1to1_bpp4_light` / `dark` / `zoom135_light` / `compose_frame` (2k×1k) | `262.1 µs` / `96.6 µs` / `524.3 µs` / `940.1 µs` |
| `highlight` — `gesture_pts100_lines200` / `sorted_pts100_lines200` / `sorted_marquee_200` | `28.69 µs` / `9.78 µs` / `107.3 ns` |
| `prefetch` — `burst_misses` / `burst_time_to_resident` | `42.68 ms` / `36.04 ms` |

---

### 3. TCL NXTPaper 11 Plus (9469X) — `pdf_bench` 5-Run Sweep (`tools/adb-bench.sh`, pantalla ON)

| Flujo medido (mediana de 5 corridas) | Métrica |
|---|---|
| `dense_textbook.pdf` (93p) — open / render1x / render2x | `0.50 ms` / `12.47 ms` / `42.89 ms` |
| `scanned_pages.pdf` (30p) — open / render1x / render2x | `0.19 ms` / `31.13 ms` / `122.13 ms` |
| `scientific_paper.pdf` (12p) — open / render1x / render2x | `0.20 ms` / `10.31 ms` / `38.95 ms` |
| `large_document.pdf` (500p) — open / render1x / render2x | `0.32 ms` / `13.15 ms` / `44.00 ms` |
| `pdf_bench` sweep — `PEAK_RSS_KB` (mediana) | `24,696 KB` (`24.1 MiB`) |
| `zoom large p0` — `scale2x` / `scale4x` / `rerender1` / `rerender2` | `71.91 ms` / `285.27 ms` / `14.72 ms` / `55.03 ms` |
| `zoom dense p0` — `scale2x` / `scale4x` / `rerender1` / `rerender2` | `72.07 ms` / `285.81 ms` / `14.95 ms` / `56.89 ms` |
| `pdf_bench` sweep en `mixed_stress.pdf` (CLI binario solo cubre 4 nombres fijos) | `SIN MEDIR` (perfilado en la app real abajo) |

> **Hallazgo crítico de `zoom`:** en ARM64, el escalado CPU `scale_bitmap` a 2× (`71.9 ms`) y 4× (`285.3 ms`) es **5× más lento** que re-renderizar la página entera desde MuPDF (`14.7 ms` / `55.0 ms`).

---

### 4. TCL NXTPaper 11 Plus (9469X) — App Real `com.pdflector.app` (pantalla ON, 120 Hz activo)

#### 4.1 Cold Start (`am start -W`, `LaunchState: COLD`)
| Flujo medido | Métrica (`TotalTime` / `WaitTime`) |
|---|---|
| `ACTION_VIEW` `scientific_paper.pdf` (3 corridas) | `645 ms`, `649 ms`, `653 ms` — **mediana `649 ms`** (`WaitTime`: `664 / 701 / 717 ms`) |
| `ACTION_VIEW` `large_document.pdf` (500p, 1 corrida) | `631 ms` (Tap) / `759 ms` (tras switch) |
| `ACTION_VIEW` `mixed_stress.pdf` (80p, 2 corridas) | `686 ms`, `692 ms` (`WaitTime`: `718 ms`) |
| Launch normal directo a Library aislada (2 libros sintéticos, sin `folder_sync`) | `633 ms` (`WaitTime`: `648 ms`) |
| Relaunch normal restaurando `mixed_stress.pdf` @ pág. 79 Continuo | `654 ms` |

> **Causa medida en `logcat`:** en arranque frío, `recreate_surface` destruye y recrea la `EGLSurface` y los Dual FBOs (`1440×2200`) **3 veces seguidas** al mismo tamaño, y `library.json` se carga 2 veces.

#### 4.2 Memoria en Reposo (Idle Viewer, `dumpsys meminfo com.pdflector.app`)
| Flujo medido | `TOTAL PSS` | `TOTAL RSS` | `Native Heap` | `Graphics` (`EGL` + `GL`) |
|---|---|---|---|---|
| `scientific_paper.pdf` (12p) idle | `187,529 KB` (`183.1 MiB`) | `302.5 MiB` | `57.7 MiB` | `83.3 MiB` (`28.2` + `55.1`) |
| `large_document.pdf` (500p) idle | `188,582 KB` (`184.2 MiB`) | `303.5 MiB` | `58.7 MiB` | `83.4 MiB` |
| `mixed_stress.pdf` (80p) idle | `182,790 KB` (`178.5 MiB`) | `297.8 MiB` | `52.9 MiB` | `83.3 MiB` |
| Library aislada (2 libros sintéticos) idle | `180,226 KB` (`176.0 MiB`) | `295.5 MiB` | `46.5 MiB` | `87.7 MiB` (`30.2` + `57.5`) |

#### 4.3 Estrés de Scroll Continuo (30 swipes rápidos, 120 Hz activo)
| Flujo medido | `frame p95` (`gpu::pipeline`) | `dumpsys gfxinfo` | `TOTAL PSS` post-scroll | `TOTAL RSS` | `Native Heap` | `Graphics` (`EGL` + `GL`) | `dlcache` (`logcat`) |
|---|---|---|---|---|---|---|---|
| `mixed_stress.pdf` (80p, 30 swipes, 1,680 frames) | **`14.5 – 14.6 ms`** (pasa 60 Hz, **falla 120 Hz `8.33 ms` por +75%**) | `0 frames` (`GameActivity` renderiza por `EGLSurface` directo sin HWUI) | **`461,767 KB` (`450.9 MiB` / `461.8 MB`)** (**2.3× sobre techo 200 MB**) | `586.8 MB` | **`246.4 MB`** (`+192.3 MB` vs idle) | **`164.9 MB`** (`GL mtrack`: `56.4 → 127.7 MB`, `+71.3 MB`) | `entries=16 bytes=564,723 peak=564,797 hits=0 misses=80 evictions=64` |
| `large_document.pdf` (500p, 30 swipes, 2,160 frames) | **`11.8 – 12.9 ms`** (pasa 60 Hz, **falla 120 Hz `8.33 ms` por +42–55%**) | `0 frames` (EGL directo) | **`291,856 KB` (`285.0 MiB`)** (**+42.5% sobre techo 200 MB**) | `406.7 MiB` | `75.6 MiB` (`77,372 KB`) | **`167.6 MiB`** (`EGL 36.3` + `GL 131.3 MiB`) | `entries=16 bytes=309,132 peak=309,132 hits=0 misses=84 evictions=68` |

> **Diagnóstico clave (evidencia combinada):**
> 1. `dlcache` (`PR-03`) acota bien sus display lists (`0.31–0.56 MB`), pero **MuPDF mantiene un `fz_store` global sin techo efectivo (256 MiB por defecto)** que retuvo `+192.3 MB` de imágenes/recursos decodificados en `mixed_stress.pdf`.
> 2. `PageSlots` (`gpu/textures.rs:273-283`) crea/destruye texturas GL nuevas en cada página que entra/sale del viewport en Continuo (`glGenTextures` + `glDeleteTextures`), disparando `GL mtrack` en **`+71–75 MB`** por retención diferida en Mali-G57 MC2.

#### 4.4 Test de Fuga de Memoria — Ciclos Library ↔ Viewer (`scientific_paper.pdf`, mismo PID `6888`, 8 viajes completos confirmados en `logcat`)
| Ciclo | `TOTAL PSS` | `TOTAL RSS` | `Native Heap` | `EGL mtrack` | `GL mtrack` | `Graphics` total |
|---|---|---|---|---|---|---|
| Ciclo 0 (Library idle) | `180,226 KB` (`176.0 MiB`) | `295.5 MiB` | `46.5 MiB` | `30.2 MiB` | `57.5 MiB` | `87.7 MiB` |
| Ciclo 1 (tras vuelta a Library) | `246,250 KB` (`240.5 MiB`) | `364.0 MiB` | `50.3 MiB` | `36.3 MiB` | `100.2 MiB` | `136.4 MiB` |
| Ciclo 5 | `273,277 KB` (`266.9 MiB`) | `390.4 MiB` | `61.8 MiB` | `44.3 MiB` | `95.6 MiB` | `139.9 MiB` |
| Ciclo 10 (pico) | `290,885 KB` (`284.1 MiB`) | `407.6 MiB` | `70.5 MiB` | `44.3 MiB` | `103.4 MiB` | `147.7 MiB` |
| Ciclo 15 (8 aperturas+cierres) | `261,429 KB` (`255.3 MiB`) | `378.9 MiB` | `51.0 MiB` | `44.3 MiB` | `105.0 MiB` | `149.3 MiB` |

- **Retención neta medida (Ciclo 0 → Ciclo 15):** `+79.3 MiB` PSS (`+9.9 MiB/viaje` promediado sobre 8 aperturas), dominado por `Graphics` (`+61.6 MiB`: `GL mtrack +47.4 MiB`, `EGL mtrack +14.1 MiB`) y `Native Heap` pico (`+24.0 MiB` en ciclo 10, parcialmente liberado a `+4.6 MiB` en ciclo 15). Confirmado en código: `enter_library()` (`reader/library.rs:47-96`) no suelta `self.doc`, `page_geometry_cache`, `scroll_geoms` ni `annotations`.

#### 4.5 Flujos No Medidos en esta Pasada
| Flujo | Estado |
|---|---|
| Latencia motion-to-photon de tinta húmeda con T-Pen físico (`IN-01`) | `SIN MEDIR` (requiere lápiz físico + cámara high-speed / verificación humana) |
| Escaneo incremental `folder_sync` sobre corpus sintético en tablet | `SIN MEDIR` (protección de privacidad D3: no se ejecutó sobre `synced-pdfs` personal) |
| `frame p50 / p99` (pipeline actual solo emite `p95` en `logcat`) | `SIN MEDIR` (`gpu/pipeline.rs` solo loguea `p95`) |
