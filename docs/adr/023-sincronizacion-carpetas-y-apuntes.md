# ADR-023 — Sincronización de carpetas y publicación de apuntes

- Fecha: 2026-10-06.
- Estado: diseño aprobado por el propietario en la conversación; implementación
  y validación descritas en `ESTADO_ACTUAL.md` y `docs/benchmark-results.md`.

## Problema

El propietario organiza material universitario en una carpeta con subcarpetas.
Quiere tener sus PDFs disponibles sin conexión en la TCL y leer en el ordenador
los apuntes realizados en PDFLector. Los dispositivos pueden estar en redes
distintas y no coincidir encendidos. PDFLector conserva las anotaciones en
sidecars SQLite privados, identificados por rutas locales diferentes entre
plataformas; copiar PDFs no transmite esas anotaciones.

## Decisión aprobada

pCloud transporta los archivos. FolderSync descarga los originales a una
carpeta de entrada Android y sube las publicaciones. PDFLector conserva copias
privadas de los originales, guarda anotaciones en orden y publica PDFs con
anotaciones estándar y respaldos JSON portables bajo `_PDFLector`.

La asignatura y ruta relativa identifican la ubicación lógica. SHA-256 identifica
la versión de contenido; la identidad privada combina ambas. Cambiar el original
crea otra copia, conservando la versión anotada anterior. Los archivos con igual
contenido en carpetas distintas mantienen anotaciones independientes.

La biblioteca muestra las rutas relativas. El original del ordenador sigue en
su ubicación; su copia publicada se abre en `_PDFLector/Apuntes/<ruta relativa>`.
Los respaldos están en `_PDFLector/Respaldo`. Se excluye `_PDFLector` de la
importación para que los PDFs publicados no vuelvan a entrar como originales.

La carpeta concreta del propietario también contiene proyectos de programación.
El procedimiento del ordenador utiliza una copia selectiva de PDFs que conserva
el árbol visible y excluye carpetas ocultas, herramientas y entornos Python.
No se sube todo el árbol universitario mediante una sincronización indiscriminada.
La automatización devuelve las publicaciones a `_PDFLector` dentro del origen.

## Guardado y publicación

Un worker Android serializa los guardados SQLite. Las instantáneas pendientes
se agrupan por documento con límites explícitos; la UI no espera disco, MuPDF
ni red. La exportación espera una pausa de edición o una petición de flush.
Los journals privados permiten reintentar trabajo pendiente tras reiniciar.
La publicación siempre parte de la copia original inmutable y usa archivos
temporales; repetirla no añade dos veces las anotaciones.

La importación recorre el árbol con límites de tamaño, profundidad y entradas,
procesa lotes y omite relecturas de archivos cuyo metadata no cambia. Las
operaciones externas pertenecen a los sincronizadores. No se sincronizan bases
SQLite abiertas, preferencias del dispositivo ni la biblioteca completa.

pCloud y FolderSync administran sus credenciales. PDFLector no incorpora claves
ni tokens del proveedor. `sha2` se añade al núcleo para SHA-256 con licencia
MIT/Apache-2.0. Los formatos de manifest y respaldo tienen versión de esquema.
La configuración de raíces y los estados del ordenador viven fuera de Git.

## Alternativas consideradas

- Sobrescribir el original con el PDF anotado: mezclaría el material de origen,
  las publicaciones y posibles modificaciones del ordenador.
- Sincronizar sidecars entre los clientes PDFLector: exige identidad compartida,
  protocolo de conflictos y compatibilidad de clientes; tampoco permite leer
  apuntes con un lector PDF habitual.
- Conexión propia a pCloud: añade autenticación, transferencia y scheduling a
  la aplicación; se reutilizan los clientes externos para este alcance.
- Syncthing: exige disponibilidad simultánea de dispositivos o un tercer
  dispositivo conectado. pCloud ya instalado permite recoger material después.

## Alcance y consecuencias

La tablet es la autora de los apuntes publicados. La edición de copias con otro
lector del ordenador no se incorpora al modelo de anotaciones de PDFLector.
Los originales y las versiones anotadas no se borran automáticamente. Las
copias ocupan espacio adicional; los límites no constituyen cuotas contratadas
ni resultados de rendimiento. El trabajo de fondo está sujeto a Android y a la
conectividad; no se garantiza llegada instantánea mientras el dispositivo duerme.

Las anotaciones PDF estándar pueden presentar diferencias respecto al trazo
vectorial de la tablet. La apariencia, especialmente grosor, transparencias y
páginas rotadas, debe comprobarse con el lector real del ordenador.

## Reversión y evidencia

Desactivar la configuración de PDFLector y detener los pares de FolderSync y
el servicio del ordenador conserva todos los documentos, sidecars y PDFs
publicados. No exige migrar las anotaciones locales existentes.

La evidencia de compilación, pruebas y TCL se registra en
[`../benchmark-results.md`](../benchmark-results.md). El procedimiento repetible
está en [la skill master-sync](../../.agents/skills/master-sync/SKILL.md).
