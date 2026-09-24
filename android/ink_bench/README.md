# Ink Bench

Spike Android aislado para comprobar el camino de presentación de baja
latencia: un `SurfaceView` transparente como capa wet y
`GLFrontBufferedRenderer` de AndroidX Graphics Core como puente de front buffer
y multi buffer. No abre PDFs, no persiste trazos y no usa Google Ink.

## Alcance técnico

- `applicationId`: `com.pdflector.inkbench`.
- `minSdk`: 29; `compileSdk`: 35.
- `androidx.graphics:graphics-core:1.0.4`, versión estable fijada.
- `MainActivity` Kotlin con `FrameLayout`: vista dry de control debajo y
  `SurfaceView` wet transparente encima.
- Solo se consumen las muestras que entrega `MotionEvent`: punto actual y
  muestras históricas. No hay `MotionEventPredictor`, extrapolación ni
  geometría futura. En API 34+ se leen los timestamps originales en
  nanosegundos; en versiones anteriores se usa el tiempo en milisegundos.
- `StrokeSession` mantiene una generación por gesto y un ring diagnóstico
  acotado a 512 segmentos. Cada muestra aceptada produce únicamente el
  segmento causal `previo → actual` (el `Down` es degenerado); el ring no se
  usa para reconstruir el commit y nunca reutiliza segmentos enviados a
  AndroidX. Sus expulsiones son `diagnosticEvictions`: solo cuentan referencias
  retiradas de ese ring, no muestras de input perdidas.
- `InkGlRenderer` convierte cada segmento causal, incluido el punto `Down`, en
  una quad de ancho fijo aproximado de 10 px y la dibuja con
  `GL_TRIANGLE_STRIP`; reutiliza un `FloatBuffer` directo de capacidad fija.
  `Cancel`, `onPause`, destrucción de superficie y destrucción de la actividad
  cancelan y limpian el renderer.
- `Up` llama solo a `commit()`: AndroidX consolida el segmento front activo en
  la capa multi, sin replay explícito ni duplicación.
- AndroidX entrega al callback multi-buffer solo los segmentos enviados desde
  el commit anterior. Este spike limpia esa capa y dibuja el batch del commit
  actual; no conserva los trazos de gestos anteriores, acorde con que no guarda
  anotaciones.
- Se solicita 120 Hz y la vista de control muestra el refresco efectivo
  observado por Android. También muestra contadores de eventos de input,
  callbacks GL, commits, `unsubmittedInput` y `diagnosticEvictions`; los
  timestamps se mantienen en `BenchMetrics`.

## Construir y probar

Desde este directorio, con Android SDK/Java 17 y Gradle 8.11.1 disponible:

```bash
gradle :app:testDebugUnitTest
gradle :app:assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n com.pdflector.inkbench/.MainActivity
```

Para comprobar alineación de coordenadas en la TCL (requiere Python y Pillow),
reinicie la actividad, ejecute un swipe de stylus por ADB y capture la pantalla:

```bash
adb shell am force-stop com.pdflector.inkbench
adb shell am start -n com.pdflector.inkbench/.MainActivity
adb shell input stylus swipe 300 900 700 1300 500
adb exec-out screencap -p > /tmp/ink-bench-alignment.png
python3 ../../tools/ink_bench/check_alignment.py /tmp/ink-bench-alignment.png 300 900 700 1300
```

Conecte la TCL NXTPaper 11 Plus (9469X), abra la aplicación y escriba con el
lápiz. Compruebe que los segmentos aparecen sobre el fondo dry, que levantar
el lápiz produce un commit y que `Cancel`, pausa o rotación no dejan el front
buffer visible. Anote el refresco efectivo y los contadores mostrados; este
spike no exporta ni guarda esos datos.

## Evidencia y límites

Las 9 pruebas JVM y la compilación debug pasaron en host. El 23-09-2026 se
instaló la APK final de este experimento y se abrió en una TCL 9469X, Android
16, 1440×2200. La vista dry y la capa `SurfaceView` fueron visibles sin
crash; el sistema mostró una petición de 120 Hz, pero el refresco físico
observado fue 60 Hz. La memoria solo se observó en reposo, sin serie temporal.
La evidencia detallada está en
[`docs/benchmark-results.md`](../../docs/benchmark-results.md).

Después, un trazo físico reveló que la proyección vertical estaba invertida.
Se corrigió; la prueba ADB del trazo asentado pasa y el propietario confirmó
con el lápiz que la tinta aparece bajo la punta en la TCL. La latencia,
continuidad en sesiones largas, FPS y las expulsiones diagnósticas siguen
**no verificados**. Este spike no integra aún el lector productivo.
