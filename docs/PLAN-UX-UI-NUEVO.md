# Plan de Implementación UI/UX - PDFLector

## 1. Filosofía de Diseño
- **Invisibilidad**: El PDF ocupa el 100% de la pantalla. La UI flota por encima y desaparece tras 3 segundos de inactividad (Immersive Mode).
- **Hardware-First (TCL NXTPaper)**: Evitar gradientes pesados, sombras con blur difuso y animaciones complejas que castiguen la GPU y penalicen los 16.6ms.
- **Táctil (Android/Tablet)**: Elementos interactivos mínimos de 48x48dp (dedo) o 32x32dp (stylus).

## 2. Design Tokens (RICOUI Adaptado)

### 2.1 Modo Papel (Inspiración Notion / Lovable)
Pensado para emular tinta sobre papel en la pantalla mate.
- **Canvas (Fondo app):** `#F6F5F4` (Blanco cálido, reduce fatiga)
- **Superficies (Paneles/Flotantes):** `#FFFFFF` con borde hairline `#E5E5E6` (Sin sombras pesadas)
- **Tinta (Texto UI):** `#1C1C1C` (Carbón suave, no negro puro)
- **Acento Primario:** `#0075DE` (Notion Blue - botones activos, selección)
- **Anotaciones (Highlighter):** `#FDECC8` (Amarillo), `#DBEDDB` (Verde), `#D3E5EF` (Azul). Multiplicar (Blend Mode: Multiply).

### 2.2 Modo Noche (Inspiración Vercel Dark)
Pensado para lectura a oscuras y paneles OLED (futuro).
- **Canvas (Fondo app):** `#000000` (Negro puro)
- **Superficies:** `#0A0A0A` con bordes `#1A1A1A`
- **Tinta (Texto UI):** `#EDEDED` (Blanco suave)
- **Acento Primario:** `#0070F3` (Vercel Blue)

## 3. Disposición Espacial (Layout)

### 3.1 Escritorio (egui)
- **Sidebar Izquierdo (Ocultable):** 250px. Pestañas superior: Índice (TOC), Miniaturas, Marcadores.
- **Centro:** Canvas del PDF (Mupdf RenderEngine).
- **Header Flotante:** Título del documento centrado, estado de sincronización (icono), buscador.
- **Barra de Herramientas Flotante (Inferior Centro):** Estilo píldora. Botones: Puntero (Pan), Selección de texto, Bolígrafo (Freehand), Subrayador, Goma.

### 3.2 Tablet Android (Slint - Fase futura, reglas aplicables)
- **Tap central:** Muestra/Oculta Top App Bar (Título/Volver) y Bottom Bar (Herramientas).
- **Gestos:** Swipe horizontal para pasar página, Pinch-to-zoom fluido.
- **Contextual Menu:** Al seleccionar texto, popup flotante (Copiar | Subrayar | IA: Resumir | IA: Explicar).

## 4. Fases de Ejecución para Agentes

- [x] **Fase 1**: Escribir este plan.
- [ ] **Fase 2 (Actual)**: Implementar `ThemeEngine` (Tokens) en `pdf_core` y `pdf_app`.
- [ ] **Fase 3**: Construir el "Shell" en `egui` (Sidebar colapsable, Top bar invisible, Canvas central).
- [ ] **Fase 4**: Barra de herramientas flotante tipo píldora (Floating Toolbar).
- [ ] **Fase 5**: Menú contextual de selección (IA + Anotaciones).