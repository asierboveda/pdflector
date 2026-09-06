// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Asier Bóveda

//! Pipeline de presentación GPU (Fase 2 de PLAN-PARIDAD-STYLUS-NATIVO,
//! ADR-006; Tarea 2.7): contexto EGL/GLES2 sobre el ANativeWindow del visor.
//! La página renderizada por MuPDF se sube como textura SOLO cuando cambia
//! (cambio de página o re-render nítido); la tinta (trazos guardados +
//! gesto en curso + tramo predicho) se dibuja como geometría vectorial
//! (quads con AA analítico en el fragment shader); los overlays (chrome,
//! toolbar, sheet, menús, badge, toast, cursor de goma) son quads
//! texturizados de los bitmaps Canvas+JNI que `draw::render_*` ya genera.
//! Present con `eglSwapBuffers` (spike 1: p50 0.17 ms vs 3.75 ms del
//! lock+post).
//!
//! PRODUCTOR ÚNICO (Tarea 2.7): el visor, la biblioteca y el picker
//! presentan TODOS por este EGL — la surface vive toda la vida de la
//! ventana y nunca se suelta al cambiar de modo (una ANativeWindow admite un
//! solo productor de BufferQueue; alternar el lock CPU de los blits SW con
//! la surface EGL agotaba el slot y `eglCreateWindowSurface` fallaba con
//! EGL_BAD_ALLOC 0x3003 en cada vuelta Library→Viewer). Los planos
//! cacheados de Library/Picker se suben como texturas dedicadas solo cuando
//! su versión cambia; el lock CPU (`ANativeWindow_lock`) queda únicamente
//! como fallback cuando no hay EGL (`gpu.is_none()` o surface sin crear).
//!
//! FFI EGL/GLES2 propio (declaraciones de las APIs públicas de Khronos;
//! licencia de este fichero, no de los headers): los crates de bindings
//! (`khronos-egl`) exigen pkg-config o `dlopen("libEGL.so.1")` — nombre que
//! no existe en Android (`libEGL.so`) — así que el spike ya validó este FFI
//! directo en la TCL.

// Clave de invalidación de la capa Dry en módulo puro (sin FFI/GL, testeable en host).
mod dry_key;
pub(crate) use dry_key::DryKey;

// Política de presupuesto LRU por bytes de las texturas de overlay en módulo
// puro (sin FFI/GL, testeable en host): evicción y bytes NO dependen de GL.
mod ovl_budget;
pub(crate) use ovl_budget::{OVL_BYTE_BUDGET, OvlBudget};

// Submódulos del pipeline GPU (Tarea 4.1, extraídos de este mod.rs):
// declaraciones FFI EGL/GLES2, shaders, ciclo de vida de surface/contexto,
// texturas y render/present. Este fichero queda como cordón de módulos y
// re-exports para mantener la API pública `crate::gpu::X`.
mod ffi;
mod pipeline;
mod shaders;
mod surface;
mod textures;

pub(crate) use surface::Gpu;
