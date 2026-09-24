# ADR-010 — Integrar tinta causal de baja latencia en el producto Android

> **Estado:** arquitectura aprobada por el propietario el 2026-09-24;
> implementación en validación. Sustituye para el producto la decisión
> experimental de [ADR-009](ADR-009-evaluacion-tinta-causal-front-buffer.md),
> sin reescribir lo que aquella autorizaba en su momento.

## Contexto

La ruta anterior componía la tinta temporal en el mismo EGL que el PDF y
contenía predicción visual. El propietario observó un trazo móvil incómodo y
pidió explícitamente escritura sin predicciones que puedan fallar. El
experimento separado validó en la TCL la alineación geométrica de una capa
AndroidX, pero no demostraba por sí solo la calidad dentro del lector.

Se explicó y aprobó migrar el host del producto, aunque implicase cambiar su
estructura, sin convertir otra APK experimental en requisito de aprobación.

## Decisión

- La APK principal se empaqueta con Gradle/Kotlin. `PdfLectorActivity` extiende
  `GameActivity`, que aloja la superficie nativa existente y una `SurfaceView`
  transparente superior para tinta. El motor PDF, `Reader`, MuPDF y la capa
  persistente permanecen en Rust/EGL.
- El lápiz alimenta un motor causal con muestras reales, en coordenadas de
  página. La ruta húmeda presenta segmentos mediante
  `GLFrontBufferedRenderer`; la anotación persistida conserva la geometría
  aceptada por ese motor. No se dibuja trayectoria futura predicha.
- JNI comunica únicamente segmentos de superficie y acciones `commit`,
  `cancel`, `clear` y disponibilidad. Si la superficie AndroidX no acepta un
  segmento, Rust conserva su ruta wet de respaldo para ese gesto.
- Los trazos consolidados permanecen en la capa provisional hasta que una
  presentación correcta confirma que la anotación está en Dry. Una
  cancelación afecta solo al gesto activo. El cambio de página o la pérdida de
  superficie reinicializan la capa transitoria según el ciclo de vida.
- Se conserva el formato SQLite/`Stroke` existente. No se migra ni se borra
  ningún PDF o anotación del usuario.
- `minSdk` pasa a 29 por AndroidX Graphics Core. Es aceptable para el único
  dispositivo objetivo actual, la TCL 9469X. Versiones de Android inferiores
  dejan de ser compatibles con esta APK.

## Alternativas valoradas

1. Mantener el wet actual en el mismo `eglSwapBuffers`: menor coste de host,
   pero no separa la presentación del lápiz del coste del PDF.
2. Trasladar toda la aplicación a Kotlin/Canvas: aumenta el alcance y obliga
   a rehacer un visor que ya funciona, sin evidencia de beneficio adicional.
3. Conservar una APK auxiliar: útil para investigar, pero no mejora la
   experiencia del producto. No satisface el objetivo aprobado.

## Verificación y límites de evidencia

El build, la firma, la instalación conservando datos, el arranque, los gestos
con lápiz, la transición wet→dry, la cancelación, el cambio de página y el
ciclo de vida deben comprobarse por separado. El experimento anterior
demuestra alineación, no latencia ni calidad integradas. Los percentiles
objetivo de `AGENTS.md` siguen siendo presupuestos, no resultados medidos.
La evidencia obtenida se registra en `docs/benchmark-results.md`.

## Reversión

La decisión no cambia el formato de datos. Se puede volver al host anterior
recompilando una versión previa firmada con la misma clave, sin migración de
anotaciones; una desinstalación no forma parte de la reversión porque borraría
datos locales. La ruta wet nativa queda como respaldo de ejecución cuando la
capa AndroidX no está disponible.
