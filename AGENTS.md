# AGENTS.md — PDFLector

> Reglas operativas para agentes y humanos. Este fichero es la fuente única de reglas.
> **Producto**: lector de PDF para tablet Android con lápiz — `crates/pdf_android`.
> **El escritorio (`crates/pdf_app`, egui) es un banco de pruebas del core, no el producto.**
> Roadmap vigente: `docs/plan/NEXT-PLAN.md` (fases A–F). Deuda: `docs/plan/DEUDA.md`.

## Jerarquía documental

Estos son los únicos documentos vivos. Si algo no está aquí, no existe como instrucción:

| Documento | Rol |
|---|---|
| `AGENTS.md` | Reglas operativas. Manda sobre todo lo demás. |
| `docs/plan/NEXT-PLAN.md` | Único roadmap editable (fases A–F). |
| `docs/plan/DEUDA.md` | Deuda técnica y decisiones abiertas, con su medida o su `SIN MEDIR`. |
| `docs/plan/00-objetivo.md` | Norte del producto: prioridades y métricas. |
| `docs/PROYECTO.md` | Visión, plataforma y alcance de producto. |
| `docs/adr/` | Decisiones arquitectónicas (snapshot inmutable con `Estado` y `Fecha`). |
| `docs/benchmark-results.md` | **Evidencia.** Registro append-only por fecha. Nunca instrucciones. |
| `docs/legal.md` | Estado de cumplimiento de licencias. |

No hay planes paralelos, ni documentos "Pro", ni material de investigación congelado: lo que dejó de servir se borró y vive en el historial de git (`git log -- <ruta>`).

## MUST

1. **Fluidez > RAM > resto.** Medir antes de afirmar. Objetivo de frame p95 < 16.6 ms (60 fps); en la TCL NXTPaper (pantalla de 120 Hz) el objetivo ambicioso es 8.33 ms. Todo cierre de fase exige medición con **fecha + hardware + flujo medido + métrica**.
2. **`pdf_core` no depende de UI.** Nunca importa `egui`, Slint ni `pdf_android`. Lógica pura, motor tras `trait RenderEngine`.
3. **Caché LRU por bytes, prefetch ±1, el hilo de UI nunca bloquea** (worker/rayon). Anotaciones vectoriales en coordenadas de página, pintadas en capa sobre el bitmap.
4. **Español en docs, chat y commits de documentación; inglés en código y mensajes de commit de código.** El `README.md` de la portada va en inglés (repo público). Explica en 2-4 líneas cualquier crate o concepto Rust nuevo que introduzcas.
5. **Cambios mínimos.** Implementa solo lo pedido. Si detectas algo mejorable fuera de alcance, escríbelo en `docs/plan/DEUDA.md` o en un Issue; no lo hagas.
6. **Documenta en el mismo commit**: decisión → `docs/adr/`; medición → `docs/benchmark-results.md`; procedimiento repetible → `.opencode/skills/`. Si cambias un comando o un requisito de entorno, actualiza en el mismo commit `AGENTS.md`, `CONTRIBUTING.md` y el skill correspondiente: hoy divergen si no.
7. **Un Issue = una tarea con criterio de cierre medible.** El repo es la verdad de plan y diseño; GitHub Issues es la cola de trabajo. Un Issue cuya fase ya no existe se cierra, no se arrastra.

## MUST NOT

- **No `unwrap`/`expect` en código de producción de `pdf_core`** (fuera de `#[cfg(test)]`; usa `Result`). `unsafe` solo acotado y comentado. Los tests y benches pueden usar `unwrap` con `#[allow]` acotado y motivo.
- **No dependencias GPL/AGPL sin aprobación explícita.** Preferir MIT/Apache-2.0 y justificar cada crate nueva en una línea.
- **No renderizar a resolución máxima** ni conservar todas las páginas en memoria.
- **No cerrar fase ni Issue sin medición** con fecha + hardware + flujo + métrica.
- **Ninguna clave en Git.** `crates/pdf_android/groq_key.txt` y `google_key.txt` son locales y están en `.gitignore`. Los embebe `include_str!`, así que **el build los exige**: en un clon limpio crea placeholders (`echo placeholder > crates/pdf_android/groq_key.txt`, igual para `google_key.txt`) — es lo que hace el CI. Nunca subas una clave real ni la incrustes en una APK distribuible.
- **La biblioteca nunca borra un PDF automáticamente.** Solo el usuario puede borrarlo.

## Trabajo con agentes

Esta sección existe porque ya se pagó el precio de no tenerla: dos ramas arreglaron el mismo bug por separado y el repositorio acumuló seis worktrees y ramas duplicadas.

1. **Fuente de verdad: `main`.** Todo agente parte de `main` al día (`git fetch && git switch main && git pull --ff-only`). Ninguna rama de trabajo es fuente de verdad y `main` no se reescribe.
2. **Un worktree = un agente = una tarea.** Cada agente trabaja en su propio worktree (`git worktree add ../pdflector-<tarea> -b feat/<tarea>` o el equivalente de Orca). Prohibido que dos agentes compartan worktree o que alguien trabaje sobre una rama ya integrada.
3. **Antes de empezar:** `git worktree list` + `git status` de los worktrees activos. Si tu rama está integrada en `main` o va más de 20 commits por detrás, se borra antes de tocar nada.
4. **Propiedad de fichero.** Ningún agente edita un fichero que otro tenga modificado sin commitear. Si dos tareas necesitan el mismo fichero, se serializan o se para y decide el dueño.
5. **Al terminar:** commit atómico en tu rama, integrar en `main` (merge o cherry-pick), y luego `git worktree remove` + `git branch -d`. Lo que no esté integrado en 7 días pasa a tag `archive/<tema>` y su worktree se borra.
6. **Un solo integrador por turno.** Quien abre la rama resuelve su merge. Prohibido mergear a `main` dos agentes a la vez.
7. **Paralelizar por ficheros disjuntos.** Si dos tareas pueden tocar el mismo fichero, el reparto está mal hecho: reasigna dominios antes de lanzar, no después.
8. **Contexto compartido por fichero, no por prompt.** Todo lo que un agente necesite saber se pasa como ruta (`local://…`, `docs/…`), nunca pegado en el mensaje.

## Arquitectura

```
crates/pdf_core      # motor MuPDF + caché LRU + prefetch + zoom + anotaciones + store SQLite
                     # + export + sync + ai + arxiv + theme. SIN UI.
crates/pdf_android   # PRODUCTO: app Android nativa (android-activity + EGL/GLES2 + JNI).
                     # gpu/ (pipeline wet/dry), ink/, reader/, draw/, input/, thumbs, discover.
crates/pdf_app       # banco de pruebas del core en escritorio (egui). No es plataforma destino.
crates/pdf_bench     # criterion + sweep; corre en host.
crates/pdf_spike     # experimentos puntuales (predicción de trazo). No entra en el APK.
docs/adr/            # ADR-001 MuPDF/AGPL · ADR-005 Android nativo (sustituye a 004)
                     # ADR-006 stylus EGL · ADR-007 pipeline wet/dry
```

Decisiones cerradas: motor **MuPDF** (AGPL-3.0-or-later, ADR-001, decisión mantenida); plataforma final **Android nativo** (ADR-005); presión del lápiz **no necesaria**; exportación a Markdown + PDF con anotaciones incrustadas; sincronización por **Syncthing** (congelada hasta después de v1).

## Comandos

```bash
# Core y banco de pruebas (host)
cargo test -p pdf_core                       # genera el corpus antes si falta
python3 tools/generate_corpus.py             # corpus/ (gitignored); requiere pillow + reportlab
cargo run -p pdf_app -- file.pdf
cargo bench -p pdf_bench -- --quick

# Calidad
cargo fmt --all
cargo clippy --all-targets -- -D warnings

# Android (TCL NXTPaper 11 Plus / 9469X, API 35, NDK r28)
export ANDROID_NDK_HOME=$HOME/Android/Sdk/ndk/android-ndk-r28
export PATH=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
echo placeholder > crates/pdf_android/groq_key.txt   # requisito de build (ver MUST NOT)
echo placeholder > crates/pdf_android/google_key.txt
cargo check -p pdf_android --target aarch64-linux-android      # verificación rápida
cargo apk build -p pdf_android --release --target aarch64-linux-android

# Medición en la tablet
tools/adb-bench.sh                    # sweep×5 + dumpsys + screencap + p95 de logcat + JSON
```

Procedimiento completo de despliegue y medición: `.opencode/skills/pdflector-rendimiento/SKILL.md`.

## Validación

| Carril | Qué valida | Dónde |
|---|---|---|
| Host | `cargo fmt --check`, `cargo clippy --all-targets -D warnings`, `cargo test -p pdf_core` | Local y CI |
| Android (compilación) | Que el producto compila para la tablet (`cargo check -p pdf_android --target aarch64-linux-android`) | Local y CI (job `android`, con claves placeholder) |
| Tablet física | Frame p95, render, PSS, gestos de lápiz, subrayado | **Solo en la TCL, vía `adb`.** Nunca afirmable desde host ni CI |

Los `#[cfg(test)]` de `pdf_android` no se ejecutan en host: si tocas esa crate, la verificación es la tablet.

## Definición de hecho

Un cambio está hecho cuando:

1. Compila y pasa `fmt` + `clippy` + `cargo test -p pdf_core`.
2. Si toca render, caché o entrada: **medido en la TCL** con fecha, hardware, flujo y métrica, registrado en `docs/benchmark-results.md`.
3. Cumple la tabla de escenarios de memoria (medido el 2026-09-06/07 en la TCL):

| Escenario | Techo declarado | Último valor medido | Estado |
|---|---|---|---|
| Arranque (PSS) | < 150 MB | 118 MB | ✅ |
| Reposo (PSS) | ≤ 180 MB | 174-178 MB | ✅ |
| Tras 15 ciclos Library→Viewer (PSS pico) | ≤ 200 MB | 232 MB → 287 MB | ❌ deuda abierta |

El umbral único de 150 MB para todo dejó de ser la regla: ahora se mide por escenario. El pico tras ciclos es deuda declarada en `docs/plan/DEUDA.md`, no un dato que se pueda ignorar.

4. La documentación afectada se actualizó en el mismo commit, y no queda ninguna referencia a ficheros borrados.

## Flujo pro

- **Una sola raíz canónica**: `~/Projects/pdflector`. Worktrees temporales de agentes en `~/orca/workspaces/pdflector/<tarea>`. Excepciones de ruta: `~/Android/Sdk`, `/tmp`.
- **Skills propios** → `.opencode/skills/<nombre>/SKILL.md` (versionados). **Skills de ecosistema** → `.agents/skills/` (gitignored, reproducibles desde `skills-lock.json` con `npx skills add`).
- **Nada de documentación zombie**: lo que no sirva para una decisión futura se borra. El historial de git es el archivo.
