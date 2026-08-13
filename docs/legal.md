# Legal / licencias — estudio preparatorio (para cuando haya versión)

> Documento de investigación. **No es una decisión tomada**: es el análisis para
> ejecutar el "tema legal" pendiente (SPDX, AGPL-3.0-or-later, atribución MuPDF)
> cuando el autor quiera publicar/distribuir la primera versión.
> Estado del repo a la fecha: LICENSE = AGPL-3.0 (texto estándar FSF "Version 3"),
> sin cláusula "or later" explícita; motor = MuPDF vía crates `mupdf`/`mupdf-sys`
> 0.8 (ambos declaran `AGPL-3.0` en su Cargo.toml).

## 1. Punto de partida

- El repo ya tiene `LICENSE` = **GNU AGPL v3** (texto oficial, "Version 3, 19
  November 2007"). Sin una nota "or later" aparte, en SPDX equivale a
  **`AGPL-3.0-only`**.
- El motor **MuPDF** (Artifex) se distribuye bajo **AGPL-3.0-or-later** (las
  cabeceras de sus fuentes dicen "either version 3 … or any later version") o
  licencia comercial. Se enlaza estáticamente (`mupdf-sys` compila el C dentro
  del binario/APK).
- Los bindings Rust `mupdf`/`mupdf-sys` declaran `AGPL-3.0` en su Cargo.toml.

## 2. Las tres piezas del trabajo pendiente

### A. Identificador SPDX en cada fichero fuente

Estándar **REUSE** (reuse.software): cada fichero con una cabecera, p. ej.:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 <Autor>
```

- Aplicar a `*.rs`, `*.toml`, `*.py`, `*.sh` y `*.md` (los `.md` opcionales).
- Alternativa mínima (sin REUSE completo): cabecera `SPDX-License-Identifier`
  en los `.rs` + un `LICENSES/` con el texto AGPL + un `REUSE.toml`. Para un
  repo pequeño, REUSE es factible y queda limpio.

### B. Variante de la licencia: `-only` vs `-or-later`

- Hoy equivale a **AGPL-3.0-only**.
- **Recomendación: pasar a `AGPL-3.0-or-later`**:
  - Coincide con la variante real de MuPDF (-or-later), lo que evita fricción
    de compatibilidad de copyleft.
  - Es la opción más común y "forward-compatible" (deja al FSF actualizar).
  - Cambio concreto: añadir al final de `LICENSE` la nota estándar y usar
    `SPDX-License-Identifier: AGPL-3.0-or-later`.
- La decisión es del autor (copyleft ya asumido en ADR-001; esto solo fija la
  variante).

### C. Atribución a MuPDF / terceros

- AGPL exige conservar los avisos de copyright. MuPDF = "Copyright (C) Artifex
  Software, Inc." (y su licencia AGPL).
- Añadir un fichero **`NOTICE`** (o `THIRD_PARTY_NOTICES.md`) con:
  - MuPDF (Artifex) — AGPL-3.0-or-later, enlace a mupdf.com y a su COPYING.
  - Crates de terceros y sus licencias (serde, rusqlite, notify, lru, egui,
    android-activity, ndk, criterion, …) — todos permisivos (MIT/Apache-2.0),
    compatibles con AGPL (regla AGENTS.md §3 ya verificada al añadirlos).
- En `README.md` una sección breve "License / third-party".

## 3. Cumplimiento al distribuir (AGPL §4–§7)

- Ofrecer el **código fuente** correspondiente: publicación en GitHub ya lo
  satisface (enlace al repo + commit exacto del build).
- Incluir el **texto de la AGPL** en el paquete (en un APK: en assets o en la
  pantalla de "About").
- **Sin restricciones adicionales** (sin DRM, sin prohibir redistribución).
- En la app Android: una pantalla "About / Licencias" con AGPL + NOTICE es lo
  más limpio (Fase 6, cuando exista la UI final).

## 4. Checklist para ejecutar (cuando el autor lo pida)

1. [ ] Decidir `AGPL-3.0-or-later` (recomendado) vs `-only`.
2. [ ] Actualizar `LICENSE` con la variante elegida (nota "or later" si procede).
3. [ ] Añadir cabeceras SPDX a todos los ficheros fuente (REUSE o mínimo `.rs`).
4. [ ] Añadir `NOTICE` con atribución MuPDF/Artifex + terceros.
5. [ ] Añadir copyright del autor (`Copyright (C) 2026 …`).
6. [ ] Revisar `cargo metadata` → licencias de TODAS las deps compatibles (regla
   AGENTS.md §3) y reflejarlo en NOTICE.
7. [ ] Pantalla "About/Licencias" en la app (Fase 6).
8. [ ] Antes del push público: confirmar que el repo público + LICENSE + NOTICE
   cumplen AGPL (fuente disponible, texto incluido, sin restricciones).

## 5. Riesgos / notas

- **No añadir dependencias copyleft adicionales** (GPL/LGPL/AGPL) sin decisión:
  ya hay una (MuPDF); otra podría complicar la combinación. Regla ya en
  AGENTS.md §3.
- La variante "-only vs -or-later" no cambia que el proyecto es copyleft (ya
  decidido en ADR-001); solo fija la letra pequeña.
- Si algún día se quiere una licencia más permisiva habría que **sustituir
  MuPDF** (es AGPL) — fuera de alcance por ADR-001.
