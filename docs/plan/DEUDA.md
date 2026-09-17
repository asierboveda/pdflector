# Deuda Técnica y Backlog de Arquitectura — PDFLector

Inventario consolidado de deuda técnica, mediciones pendientes y propuestas arquitectónicas vivas. Cada elemento se clasifica explícitamente como `medido` (con datos de hardware) o `idea sin medir`.

---

## 1. Deudas medidas de rendimiento y estabilidad

### D-MEM-01: Display lists sin cota en `MupdfDocument`
- **Clasificación**: `medido`
- **Evidencia**: Medido en TCL 9469X (2026-09-07). Cada nueva página visitada añade entre +6 y +7 MB al proceso reteniendo display lists en la estructura nativa de MuPDF. El consumo escala de 190 MB a 330 MB tras navegar decenas de páginas.
- **Acción requerida**: Implementar política de evicción LRU para soltar display lists de páginas lejanas a la ventana actual.

### D-MEM-02: PSS pico elevado en ráfaga de navegación
- **Clasificación**: `medido`
- **Evidencia**: Medido en TCL 9469X (2026-09-07). En 15 cambios de página consecutivos rápidos, el PSS pasa de 234.7 MB a 287.5 MB (arranque 118 MB, reposo 174–178 MB). Aunque a +20 s y +40 s el consumo se estabiliza sin fuga descontrolada, el pico viola el techo de 200 MB tras ciclos interactivos.
- **Acción requerida**: Limitar la retención de texturas y estructuras nativas tras navegación rápida.

### D-HAR-01: Ausencia de gate de benchmark en CI (A4)
- **Clasificación**: `medido`
- **Evidencia**: Inspección de `.github/workflows/ci.yml`. No existe ningún paso que ejecute `cargo bench` ni verifique regresiones en el umbral de `composite < 5ms`.
- **Acción requerida**: Añadir job de benchmark en CI con criterio de fallo ante regresión de rendimiento.

### D-HAR-02: Medición de interacción masiva con stylus en tablet (A5)
- **Clasificación**: `medido` (estado: bloqueado para stylus físico)
- **Evidencia**: Medido parcialmente en TCL 9469X (sweep sintético de 5 pasadas ejecutado en 2026-09-04). La prueba de 200 trazos simultáneos y 100 gestos de highlight sobre la tablet no ha podido automatizarse por requerir eventos de stylus físico que el driver rechaza en inyecciones multitáctiles sintéticas.
- **Acción requerida**: Diseñar arnés de prueba de inyección compatible con el pipeline de stylus o sesión de validación guiada con hardware.

### D-DRW-01: Cierre de rendimiento de pintado en hardware real (Cierre Fase C)
- **Clasificación**: `medido` en host / `idea sin medir` en tablet
- **Evidencia**: En host x86_64 se midieron 4.39 ms directo y 2.39 ms cacheado con `StrokeCache` a resolución 1440×2200. En la tablet TCL 9469X el objetivo de pintado en vivo < 8 ms p95 con 200 trazos activos permanece `SIN MEDIR`.
- **Acción requerida**: Medir con 200 trazos reales en el FBO wet de la GPU de la tablet.

### D-LIB-01: Carga en frío del visor y escala a 256 libros (E3 y cierre Fase E)
- **Clasificación**: `medido`
- **Evidencia**: Medido en TCL 9469X (2026-09-07). Con 11 libros el scroll de rejilla cumple holgadamente (p95 10.0 ms < 16.6 ms). Sin embargo, el ensayo con 256 libros está `SIN MEDIR`, y el tiempo de apertura del visor en cold-start se midió en 349 ms (213 ms apertura de PDF + 73 ms InitWindow), superando el objetivo de < 200 ms.
- **Acción requerida**: Optimizar apertura asíncrona o diferida del motor para alcanzar < 200 ms y validar catálogo masivo.

---

## 2. Backlog técnico y propuestas de diseño

### Persistencia y anotaciones

- **DEC-ANN-01: Decisión arquitectónica de persistencia (sidecar vs. dual-write)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Decidir entre mantener el fichero `.pdflector.json` lateral (sidecar) como fuente primaria o implementar escritura dual directa en el binario PDF mediante guardado incremental (`incremental save`) y patrón atómico temporal + renombrado (estilo Okular). Requiere evaluar orden canónico de coordenadas de esquinas (`ll`, `lr`, `ul`, `ur`).
- **ANN-01: Anclaje de texto robusto en 2 columnas y des-guiado**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Refinar la asignación de líneas en PDFs científicos con maquetación de doble columna para evitar saltos de columna erróneos durante el gesto manual.
- **ANN-02: Nota textual asociada a Highlight**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: El enum actual de anotaciones no incluye campo de notas de texto adjuntas al resaltado (`Annotation::Highlight` carece de `note: Option<String>`).
- **ANN-03: Agrupación de trazos en `set_ink_list`**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Agrupar trazos cercanos en el tiempo o espacio en una sola estructura lógica de anotación ink para optimizar serialización y exportación estándar.
- **ANN-04: Soporte de `Quad` para texto rotado**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Permitir cajas de selección orientadas (cuadriláteros de 8 coordenadas) para PDFs con texto en ángulo o vertical.
- **ANN-05: Vista previa de exportación de anotaciones**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Interfaz ligera para previsualizar el PDF resultante con anotaciones horneadas antes de compartirlo.

### Zoom, navegación y gestos

- **NAV-01: Snap-back animado al soltar límites de zoom**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Animación suave de recuperación cuando el usuario hace un pinch-to-zoom que excede la escala mínima o máxima permitida.
- **NAV-02: Compensación de deriva de foco (`mLastScaleFocus`)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Compensar el desplazamiento indeseado del punto de pivote central durante el escalado con dos dedos.
- **NAV-03: Conservación estricta de fracciones de píxel en pan**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Evitar truncamiento prematuro de decimales en coordenadas de desplazamiento para evitar jitter visual.
- **NAV-04: Fling inercial con margen elástico (`FLING_MARGIN`)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Implementar desaceleración física inercial tras soltar un deslizamiento rápido de página.
- **NAV-05: Modos de ajuste automático a ancho de pantalla (fit-width)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Opción de zoom automático que recorta márgenes laterales en blanco para maximizar el área de lectura en pantallas mate.
- **NAV-06: Estudio del método nativo `getScaleFactor()`**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Investigar si el detector de gestos nativo de Android proporciona mejor estabilidad que el cálculo de distancia euclidiana manual.

### Biblioteca y gestión de catálogo

- **LIB-01: Caché de miniaturas persistida en disco**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: En la actualidad `ThumbCache` opera exclusivamente en memoria RAM. Guardar miniaturas prerenderizadas en disco para eliminar el coste de generación tras reiniciar la aplicación.
- **LIB-02: Portadas de texto sustitutivas (placeholder text-cover)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Mostrar carátula tipográfica elegante inmediatamente con el título del documento mientras el `ThumbWorker` genera la miniatura gráfica.
- **LIB-03: Bloqueo de libros dañados con tope de reintentos**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Marcar ficheros no procesables o corruptos para no intentar renderizar su portada en bucle en cada inicio.
- **LIB-04: Búsqueda y filtrado en worker secundario**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Mover la comparación de cadenas de búsqueda de títulos de libros fuera del hilo principal para catálogos con cientos de entradas.
- **LIB-05: Deduplicación por hash para sincronización externa (Syncthing)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Identificar libros por hash criptográfico o de contenido en lugar de ruta de fichero absoluta para preservar estado de lectura y sidecars ante cambios de ruta.

### Motor de tinta y lápiz

- **DRW-01: Grosor de trazo dinámico por velocidad de desplazamiento**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Modular el grosor geométrico del trazo en función de la velocidad del puntero, emulando la física de una pluma estilográfica.
- **DRW-02: Presets configurables de grosor de lápiz**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Actualmente `STROKE_WIDTH_PT = 2.0` es una constante fija. Añadir selector rápido de calibres (fino, medio, grueso).
- **DRW-03: Cursor visual de proximidad del stylus (hover)**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: El manejador en `motion.rs` ignora eventos `HoverMove`. Dibujar indicador visual de la posición del lápiz antes de tocar la pantalla.

### Renderizado y memoria

- **RND-01: Parche de alta resolución para región visible ampliada**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Al hacer zoom profundo, renderizar un parche nítido únicamente del área visible de la pantalla en lugar de escalar toda la página a gran resolución.
- **RND-02: Cola de renderizado priorizada con cancelación interactiva**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Cancelar inmediatamente solicitudes de decodificación de páginas previas cuando el usuario pasa páginas rápidamente hacia adelante.
- **RND-03: Manejador del evento del sistema `MainEvent::LowMemory`**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Actualmente `LowMemory` no se intercepta en el bucle de eventos. Al recibirlo, purgar inmediatamente cachés de texto, texturas inactivas y display lists.
- **RND-04: Caché de páginas renderizadas en disco**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Almacenar bitmaps de páginas complejas en almacenamiento flash para recuperación instantánea sin coste de CPU.
- **RND-05: Tiling selectivo con umbral de activación**
  - **Clasificación**: `idea sin medir`
  - **Descripción**: Dividir la página en teselas sólo cuando la resolución exceda el tamaño máximo de textura de la GPU (GLES2 `GL_MAX_TEXTURE_SIZE`), evitando overhead en escalas estándar.
