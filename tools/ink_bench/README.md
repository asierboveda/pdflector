# `ink_bench`

El analizador determinista usa únicamente la biblioteca estándar de Python
para validar resultados del benchmark de tinta. No instala APKs, no usa ADB y
no mide la tablet: procesa un flujo JSONL ya capturado. El comprobador
independiente `check_alignment.py` sí usa Pillow para revisar píxeles de una
captura ADB, pero tampoco mide latencia ni calidad de escritura.

## Comandos exactos

Desde la raíz del repositorio:

```bash
PYTHONPATH=tools/ink_bench python3 -m unittest discover -s tools/ink_bench/tests -v
python3 tools/ink_bench/analyze.py tools/ink_bench/fixtures/valid.jsonl
```

El analizador escribe JSON con claves ordenadas y estados explícitos. Devuelve
código 0 para un JSONL válido aunque el resultado sea `no_verificado`,
`inconcluso` o `rechazado`; devuelve código 2 para una entrada inválida.

## Formato `ink-bench/v1`

Cada línea es un objeto JSON con `schema_version: "ink-bench/v1"` y
`record_type`. La primera línea debe ser exactamente un registro `run`:

```json
{"schema_version":"ink-bench/v1","record_type":"run","run_id":"2026-09-23-engine-a","created_at":"2026-09-23T10:00:00Z","device":{"model":"TCL NXTPAPER 11 Plus (9469X)","android_version":"...","build":"..."},"display":{"width_px":1440,"height_px":2200,"requested_hz":120,"effective_hz":120},"thermal":{"initial_c":30.0,"final_c":31.0},"warmup_samples":20,"engines":[{"id":"engine-a","version":"..."}],"corpus":[{"id":"corpus-a","files":[{"path":"paper.pdf","sha256":"<64 hex>"}]}],"order":["engine-a/corpus-a"],"raw_files":[]}
```

`engines`, `corpus` y `order` son la preinscripción: se escriben antes de
capturar resultados y no se deducen del flujo. Cada JSONL describe exactamente
una celda: un motor, un corpus y una frecuencia efectiva. Para comparar
condiciones se generan ficheros JSONL separados; los records no llevan
`engine_id`/`corpus_id` porque no se permite mezclar celdas en un fichero.
`raw_files` solo enumera artefactos externos y puede ser una lista vacía. Todos
los PDFs/corpus y ficheros crudos que sí se enumeren deben quedar identificados
por SHA-256. Por ejemplo:

```bash
sha256sum corpus/*.pdf captures/*.jsonl > hashes.sha256
```

`analyze_file` calcula y publica `input_sha256` para el JSONL analizado. El
JSONL no puede declarar su propio hash dentro de `raw_files`.
Si se aportan mediciones externas para solicitar aprobación, `raw_files` debe
incluir el vídeo o captura de cámara con `kind: "external_camera"`, ruta y
SHA-256. La herramienta verifica el formato de ese identificador; el operador
debe conservar el archivo y comprobar que el hash corresponde a sus bytes.

Un registro `sample` contiene timestamps monotónicos en nanosegundos para
separar los tramos:

```json
{"schema_version":"ink-bench/v1","record_type":"sample","sample_id":1,"stroke_id":"stroke-0","event_ts_ns":1000000,"dispatch_ts_ns":2000000,"model_start_ts_ns":2000000,"model_end_ts_ns":3000000,"geometry_start_ts_ns":3000000,"geometry_end_ts_ns":4000000,"submit_ts_ns":5000000,"swap_start_ts_ns":5000000,"swap_end_ts_ns":5500000,"prediction_used":false}
```

El analizador publica por separado `event_to_dispatch_ms`, `app_work_ms`
(`dispatch→submit`), `model_ms`, `geometry_ms`, `event_to_submit_ms`
(`stylus→envío`) y `swap_ms`; no agrega esos tramos en una métrica única.

Un registro `frame` representa la presentación. El analizador publica
`presentation_latency_ms` (`submit→present`, solo frames presentados) y
`frame_interval_ms` entre
presentaciones reales consecutivas. `presented=false` cuenta como frame perdido
y su `present_ts_ns` no entra en la cadencia. Los huecos se estiman a partir de
los `vsync_ts_ns` de la frecuencia efectiva y se suman a los frames perdidos
declarados, sin contarlos dos veces:

```json
{"schema_version":"ink-bench/v1","record_type":"frame","frame_id":1,"vsync_ts_ns":10000000,"submit_ts_ns":10000000,"present_ts_ns":14000000,"presented":true}
```

Un registro `memory` lleva PSS en KiB y una fase (`warmup`, `stable` o
`stress`):

```json
{"schema_version":"ink-bench/v1","record_type":"memory","sample_id":1,"ts_ns":10000000,"pss_kb":180000,"phase":"stable"}
```

El registro opcional `external_latency` es la única fuente válida para
`lápiz→píxel`. Debe proceder de una medición externa y contener latencia,
`fps` de cámara e incertidumbre:

```json
{"schema_version":"ink-bench/v1","record_type":"external_latency","latency_ms":20.0,"fps":240,"uncertainty_ms":1.0}
```

Se escriben al menos 30 líneas `external_latency`, una por observación
externa; el ejemplo muestra una de ellas.

La ausencia de este registro produce `no_verificado`; el analizador nunca
infiere lápiz→píxel a partir de timestamps internos. Una captura incompleta es
un error de formato, no una aprobación.

## Protocolo de captura

1. Preinscribir motores, corpus, versiones, hashes y el orden antes de cada
   sesión. Para comparar varios motores y corpus se usa un cuadrado latino:
   con `n` condiciones, la sesión `r` ejecuta la condición `(r + c) mod n` en
   la posición `c`; se registran todas las filas ejecutadas en `order`.
2. Registrar resolución, refresco solicitado y refresco efectivo. El flujo
   admite 60 o 120 Hz efectivos; otro valor queda `no_verificado`, aunque el
   resto de cifras parezca correcto. Una sesión efectiva a 60 Hz sirve para
   diagnóstico y comparación a 60, pero no puede aprobar el objetivo de
   presentación a 120 Hz: queda `no_verificado` con razón
   `120_hz_not_measured`. No se sustituye el refresco efectivo por el
   solicitado.
3. Anotar temperatura inicial y final, y separar calentamiento (`warmup`) de
   la ventana estable. Registrar al menos 3 muestras `stable` y 1 muestra
   `stress`; si faltan, el resultado queda `no_verificado`. Repetir ciclos con
   temperatura estable; una PSS que crece estrictamente durante la ventana
   estable, ordenada por `ts_ns`, queda `inconcluso`.
4. Capturar input, presentación y PSS sin bloquear el hilo interactivo. Cada
   sample lleva `stroke_id`; no se aprueba una celda con menos de 60 trazos
   distintos y 1000 samples internos. La captura de presentación debe tener al
   menos 120 frames. La predicción se marca en cada sample; si algún sample usa
   predicción, el resultado se marca `no_verificado` para este protocolo.
5. Medir lápiz→píxel con cámara externa de al menos 240 fps y al menos 30
   observaciones. Conservar los frames crudos y su hash; informar la
   incertidumbre en milisegundos. El límite se evalúa de forma conservadora
   como `p95 + incertidumbre ≤ 25 ms`.

Los umbrales evaluados son exactamente los presupuestos de `AGENTS.md` que
corresponden a este flujo:

| Métrica | Umbral |
|---|---:|
| Trabajo de app (`dispatch→submit`) p95 | ≤ 6 ms |
| Intervalo de frame p95 (`frame_interval_ms`) | ≤ 8,33 ms |
| Intervalo de frame p99 (`frame_interval_ms`) | ≤ 16,67 ms |
| Frames perdidos | < 1 % |
| Stylus→envío de frame (`event→submit`) p95 | ≤ 8,33 ms |
| Lápiz→píxel externo p95 | ≤ 25 ms |
| PSS estable (p95 de ventana estable) | ≤ 250 MB |
| PSS pico de estrés | ≤ 350 MB |

El analizador expresa PSS en KiB (`250 × 1024` y `350 × 1024`) porque es la
unidad habitual de la fuente de captura. Todos los percentiles usan
interpolación lineal determinista y publican p50, p95 y p99.

`compare_results` solo compara runs plenamente `aprobado`. Ordena por p95 de
lápiz→píxel externo, después p95 de `event→submit` y finalmente p95 de
`app_work`. Si la diferencia en el primer criterio relevante es menor de 1 ms,
devuelve `inconcluso` en lugar de inventar un ganador. También exige el mismo
dispositivo y versión Android, resolución, refresco solicitado y efectivo, y
corpus con los mismos hashes. La versión del motor puede cambiar.

## Estados y límites

- `aprobado`: todos los umbrales aplicables pasan, existen al menos 60 trazos,
  1000 samples internos, 120 frames, 3 muestras `stable`, 1 `stress` y 30
  observaciones externas completas a ≥240 fps con una captura de cámara
  identificada por ruta y SHA-256.
- `rechazado`: existe evidencia completa que supera un umbral.
- `no_verificado`: falta evidencia externa, hay menos de 30 observaciones, el
  refresco efectivo no es 60/120, el run es de 60 Hz para el objetivo de 120 Hz
  o se usó predicción.
- `inconcluso`: la memoria estable crece o una comparación contiene un run no
  verificable.
- `empate`: dos resultados comparables tienen la misma puntuación; nunca se
  inventa un ganador.

Este directorio contiene fixtures sintéticos para percentiles, refresco
incorrecto, predicción, datos incompletos, memoria creciente y comparación
inconclusa/empate. No contienen resultados de la TCL ni deben presentarse como
mediciones reales.
