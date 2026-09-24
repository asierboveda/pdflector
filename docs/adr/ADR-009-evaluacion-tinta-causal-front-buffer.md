# ADR-009 — Evaluación aislada de tinta causal y front buffer

> **Estado:** Aceptado para experimentación (2026-09-23). La integración en
> el producto permanece pendiente de evidencia y aprobación.
>
> **Supersede:** las partes de
> [ADR-006](ADR-006-motor-stylus-baja-latencia.md) y
> [ADR-007](ADR-007-pipeline-wet-dry-ink.md) que prescriben o permiten dibujar
> predicción de trayectoria. No modifica todavía el presentador del producto
> ni su formato de anotaciones.

## Contexto

Durante la escritura con bolígrafo aparece una línea transitoria que parte de
la punta y recorre la pantalla. Desaparece al levantar el lápiz, pero hace
incómoda la escritura. La corrección del epoch de timestamps no eliminó el
síntoma.

La inspección del código verificó que el camino actual mezcla geometría
confirmada con un `predicted_pt` reemplazable, reconstruye por completo el FBO
wet y presenta mediante el mismo `eglSwapBuffers` que compone el visor. La
observación de TCL Notes mostró un `SurfaceView` BLAST independiente,
actualizaciones por rectángulos y persistencia posterior al gesto; no permitió
determinar su algoritmo interno.

El propietario prioriza una respuesta sin lag, pero rechaza predicciones que
puedan equivocarse. También aprobó evaluar una solución más compleja siempre
que se demuestre en la TCL 9469X antes de migrar el producto.

## Decisión

1. La tinta visible será estrictamente causal: solo puede representar muestras
   reales recibidas. No se usará `MotionEventPredictor`, `predicted_pt` ni otra
   extrapolación futura para cumplir objetivos de latencia.
2. Se evaluará la presentación por separado del modelado mediante una
   aplicación Android experimental con identificador distinto al producto.
   Usará una capa wet sobre `SurfaceView` y
   `androidx.graphics.lowlatency.GLFrontBufferedRenderer`; el fondo dry no se
   actualizará por cada muestra.
3. El primer motor será causal, mínimo y determinista. Compartirá un contrato
   intercambiable con futuros candidatos, pero no se conectará aún a
   `Reader`, GPU productiva o persistencia.
4. El formato existente `Stroke { points, width, color }` y los sidecars no se
   modificarán durante los experimentos.
5. Google Ink solo se evaluará después de demostrar la viabilidad de la capa
   de presentación. La comparación deberá usar su geometría real o su pila
   completa; devolver simplemente sus muestras de entrada no contará como una
   comparación de calidad del motor.
6. Las dependencias AndroidX del experimento estarán fijadas a versiones
   concretas y confinadas al módulo de prueba. Su incorporación al APK de
   producto requerirá una decisión posterior.

## Evidencia exigida

La decisión de producción se tomará con el mismo corpus, trazas, estilo,
presentación y condiciones térmicas para todos los candidatos. Se aplicarán
los presupuestos de `AGENTS.md`, entre ellos:

- trabajo de aplicación p95 ≤ 6 ms;
- frame p95 ≤ 8,33 ms y p99 ≤ 16,67 ms a 120 Hz efectivo;
- menos del 1 % de frames perdidos;
- evento de stylus → envío de frame p95 ≤ 8,33 ms;
- lápiz → píxel p95 ≤ 25 ms, medido externamente;
- PSS estable ≤ 250 MB y pico ≤ 350 MB sin crecimiento monotónico.

`screenrecord`, el retorno de `eglSwapBuffers` y los timestamps internos no
demuestran lápiz→píxel. Esa métrica requiere cámara externa de al menos
240 fps u otro instrumento equivalente.

## Consecuencias

- El experimento puede introducir Gradle/Kotlin y AndroidX sin cambiar el
  empaquetado actual de `pdf_android`.
- La presentación y el motor pueden aprobarse o rechazarse de forma
  independiente.
- Si varios motores cumplen y la diferencia no es concluyente, se elegirá el
  más simple y mantenible.
- Ningún resultado del spike se describirá como estado del producto hasta que
  se integre y verifique en la TCL.
- El dispositivo puede ignorar una solicitud de 120 Hz. Toda prueba registrará
  tanto la tasa solicitada como la efectiva y no mezclará sesiones de 60 y
  120 Hz.

## Reversión

La aplicación experimental usa otro `applicationId`, no abre PDFs personales
ni escribe anotaciones. Eliminarla no requiere migrar datos ni modificar el
APK productivo. El motor actual permanece disponible hasta que una decisión
posterior apruebe su sustitución.
