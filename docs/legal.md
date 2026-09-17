# Estado legal y de licencias — AGPL-3.0-or-later

Este documento describe el estado real de cumplimiento de licencias de PDFLector y los requisitos necesarios para la distribución pública de binarios (APK de Android).

## 1. Estado actual y base jurídica

El repositorio cumple con los requisitos legales de licenciamiento libre:

- **Licencia principal**: **`AGPL-3.0-or-later`** (GNU Affero General Public License v3 o cualquier versión posterior), declarada formalmente en [`LICENSE`](../LICENSE) (con identificador SPDX `AGPL-3.0-or-later`) y en [`README.md`](../README.md).
- **Motor de renderizado (MuPDF)**: MuPDF (Artifex Software, Inc.) se distribuye bajo `AGPL-3.0-or-later` y se enlaza estáticamente a través de los crates `mupdf` / `mupdf-sys` (ADR-001). Al adoptar PDFLector la misma variante (`AGPL-3.0-or-later`), se asegura una compatibilidad copyleft perfecta.
- **Atribuciones de terceros**: el fichero [`NOTICE`](../NOTICE) en la raíz del repositorio contiene la atribución preceptiva a Artifex Software / MuPDF y el inventario de todas las dependencias directas y transitivas notables.
- **Compatibilidad de dependencias**: todas las librerías de terceros utilizadas (bindings de Android NDK, `rusqlite`, `lru`, `serde`, `android-activity`, `criterion`, `egui`, etc.) disponen de licencias permisivas (`MIT`, `Apache-2.0` o `CC0-1.0`), plenamente compatibles con la AGPL-3.0-or-later.
- **Disponibilidad del código fuente**: el código completo reside en un repositorio público accesible, satisfaciendo el requisito de oferta de código fuente (AGPL §6).

## 2. Requisitos para la distribución de la APK

Para distribuir la aplicación compilada a usuarios finales en formato APK, restan por completar los siguientes aspectos operativos:

1. **Pantalla "Acerca de / Licencias" en la app Android**:
   - Para cumplir los términos de entrega de binarios (AGPL §4 y §5), la interfaz nativa (`crates/pdf_android`) debe incluir una vista accesible que exponga el texto de la licencia AGPL-3.0 y el contenido del fichero `NOTICE`.
2. **Gestión de claves de API de IA en compilaciones públicas**:
   - Las claves de Groq y Gemini actualmente se resuelven en compilación vía ficheros locales placeholder (ver [`CONTRIBUTING.md`](../CONTRIBUTING.md)). Las APKs públicas no deben empaquetar claves de desarrollo personales; el asistente debe solicitar las claves al usuario en tiempo de ejecución o desactivarse si no se configuran.
3. **Trazabilidad del código fuente (build info)**:
   - Incluir en la pantalla de información de la APK el hash exacto del commit de Git con el que se compiló el paquete, permitiendo a cualquier usuario contrastar el binario con el código fuente correspondiente en GitHub.
4. **Sin restricciones adicionales (AGPL §7)**:
   - La distribución no debe incorporar mecanismos de gestión de derechos digitales (DRM), firmas que impidan la instalación de versiones modificadas ni cláusulas adicionales que restrinjan las libertades de la AGPL.

## 3. Política de dependencias

- **Única dependencia copyleft**: MuPDF es la única librería con licencia copyleft del proyecto.
- **Nuevas dependencias**: cualquier futura biblioteca añadida al workspace debe emplear licencias permisivas (`MIT`, `Apache-2.0`, `BSD` o equivalente) y debe ser registrada de inmediato en el fichero `NOTICE`.
- **Prohibición de incompatibilidades**: queda prohibido incorporar dependencias bajo licencias propietarias o con cláusulas que colisionen con los términos de la AGPL-3.0-or-later.
