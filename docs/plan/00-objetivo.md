# Objetivo — PDFLector

Norte del proyecto: visión, prioridades innegociables y criterios de aceptación medibles.

## Visión

Lector de documentos PDF personal, rápido y ligero, optimizado específicamente para tablet Android con lápiz óptico (TCL NXTPaper 11 Plus, pantalla mate 1440×2200, procesador MT8781 8× Cortex-A55). Gratuito, sin anuncios, sin telemetría y desarrollado en Rust nativo sobre Android NDK/EGL/GLES2.

## Prioridades innegociables

1. **Fluidez total**:
   - Mínimo sostenido de 60 fps (tiempo de frame p95 < 16.6 ms).
   - Objetivo para interacción directa con lápiz (trazo, pan y subrayado): 120 Hz (tiempo de presentación p95 < 8.33 ms).
2. **Memoria (PSS) controlada por escenario**:
   - La aplicación no debe degradar el sistema operativo ni sufrir OOM kill por acumulación de recursos tras uso prolongado.
3. **Privacidad absoluta**:
   - Cero telemetría, cero analíticas, cero llamadas a red salvo peticiones explícitas del usuario (Discover arXiv o consulta IA configurada).
4. **Simplicidad arquitectónica**:
   - Preferencia por código directo y testeable frente a capas de abstracción innecesarias.

Si una propuesta entra en conflicto con las prioridades 1 o 2, se descarta salvo autorización explícita documentada en un ADR.

## Fuera de objetivo

- **Modulación artística de pincel por presión**: El visor captura trazos con grosor escalar uniforme conforme a la especificación estándar PDF (ISO 32000).
- **Scroll continuo vertical**: El visor opera exclusivamente por páginas fijas con cambio de página instantáneo por tap o tecla.
- **Servidores propios, sincronización en la nube o cuentas**: No hay backend ni almacenamiento remoto gestionado por el proyecto.

## Criterios de cierre (DoD) por escenario

El rendimiento y consumo se evalúan en hardware real (TCL NXTPaper 11 Plus, 9469X) según tres escenarios operativos:

### 1. Arranque en frío (Cold start)
- **Techo declarado**: PSS < 150 MB; primer frame interactivo en pantalla < 200 ms.
- **Medición real (2026-09-04 / 2026-09-07)**:
  - PSS: 52.9 MB inicial, 118 MB tras carga básica (supera el techo de memoria).
  - Primer frame en visor: 349 ms (213 ms apertura de documento + 73 ms inicialización de ventana EGL; pendiente de optimización para alcanzar < 200 ms).

### 2. Reposo (Idle tras carga de documento)
- **Techo declarado**: PSS < 180 MB estabilizado; consumo de CPU < 2% en reposo.
- **Medición real (2026-09-06)**:
  - PSS en reposo: 174–178 MB (asentado sin oscilación tras estabilización de buffers GPU).

### 3. Tras ciclos de lectura interactiva (15 a 130 cambios de página)
- **Techo declarado**: Memoria asentada sin fugas continuas tras cesar la interacción; PSS estabilizado < 200 MB.
- **Medición real (2026-09-07)**:
  - PSS base 234 MB → 287 MB tras 15 cambios de página rápidos.
  - Asentamiento a +20 s (287.5 MB) y +40 s (287.5 MB): sin fuga descontrolada, pero con un pico elevado debido a las display lists de MuPDF retenidas sin cota LRU (registrado como deuda técnica prioritaria).

### 4. Presupuesto de interacción (Frame budget)
- **Pase de página**: Medido p50 ≈ 8 ms (11 de 15 turnos entre 6 y 10 ms sobre páginas cacheadas).
- **Pan continuo con stylus**: Medido p50 3.15 ms, p95 4.19 ms (por debajo del objetivo de 8.33 ms).
- **Subrayado continuo con stylus**: Medido p50 2.8 ms, p95 3.5 ms (por debajo del objetivo de 8.33 ms).

## Cómo modificar el objetivo

Cualquier cambio en prioridades o métricas debe realizarse directamente en este documento y reflejarse en `docs/plan/NEXT-PLAN.md`.
