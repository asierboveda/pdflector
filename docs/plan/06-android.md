# Fase 6 — Aterrizaje Android

> Milestone: `Fase 6 — Aterrizaje Android` · Issue: #30 · Estado: 🟡 Spike OK, falta port completo
> Decisión: **Slint Aceptado** (ADR-004). minSdk 26, linker `aarch64-linux-android26-clang`.

## Objetivo

App Android final en uso diario en la TCL, una semana sin cuelgues ni tirones.

## Criterio de aceptación

- [x] Spike Slint 1.17.1: APK 6.4MB, 62MB PSS, input verificado (tap/swipe llegan via ALooper)
- [x] `pdf_android` nativa actual funciona (gestos, zoom, biblioteca MediaStore, sheet)
- [ ] Port completo `pdf_core` → Slint (reutiliza pdf_core intacto)
- [ ] Lápiz real validado (stylus, no solo tap de dedo)
- [ ] Medición final TCL: p95 scroll 500 pág. <16.6ms, PSS <150MB, batería

## Tareas

- [ ] Crear crate `pdf_android_slint` (o `pdf_app_slint`) con backend `backend-android-activity-06`
- [ ] Subir `minSdk 26` ya hecho en `.cargo/config.toml` y `Cargo.toml`
- [ ] Vigilar no-repaint Slint #8692/#12687/#12688 (requiere Slint >1.17.1 o parche)
- [ ] Semana de uso real

## Cómo modificar

- Si quieres descartar Slint y volver a `pdf_android` nativo (Canvas+JNI): cambia ADR-004 a Rechazado y actualiza este fichero.
- Si quieres Tauri: re-abre spike (ADRs).

## Referencias

- `docs/adr/ADR-004-ui-android.md`
- `crates/pdf_android/` (actual, Canvas)
- `.opencode/skills/pdflector-rendimiento/SKILL.md`
