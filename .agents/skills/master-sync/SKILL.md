---
name: master-sync
description: Configurar y comprobar la sincronización de una carpeta universitaria con pCloud, FolderSync y PDFLector, conservando originales y apuntes.
---

# Sincronización de material y apuntes

No registrar credenciales, PDFs personales ni nombres de documentos en Git o evidencia compartida.

## Ordenador

`tools/master-cloud-sync.py` acepta `--config /ruta/local.json` y `--once`.
El JSON local contiene tres rutas absolutas: `source_root` (carpeta original),
`cloud_root` (copia dentro del disco virtual de pCloud) y `mount_root` (punto de montaje).
El script requiere que el disco esté montado, copia solo PDFs de carpetas visibles,
excluye `_PDFLector`, directorios de desarrollo y entornos Python, conserva el árbol
relativo y devuelve `Apuntes` y `Respaldo` a `_PDFLector` dentro del origen.
Nunca borra originales ni copias por un borrado remoto. Los cambios locales en una
copia publicada se conservan con nombre de conflicto antes de recibir otra copia.

Usar configuración y estado fuera del repositorio. Un servicio systemd de usuario
puede ejecutar el script continuamente; comprueba cambios cada 30 segundos.
Leer la skill de Omarchy antes de editar la configuración del ordenador.
Para suspender el servicio personal configurado con este nombre:

```bash
systemctl --user stop pdflector-master-sync.service
```

Para quitar el inicio automático, conservando todos los archivos:

```bash
systemctl --user disable pdflector-master-sync.service
```

## Tablet

1. Instalar FolderSync y conectar la **misma cuenta** que pCloud del ordenador.
   La autenticación se hace en el dispositivo; no pedir contraseñas por chat.
2. Configurar un par pCloud → carpeta local con solo los originales PDF,
   subcarpetas incluidas, excluyendo `_PDFLector` y temporales.
3. Configurar otro par carpeta local `_PDFLector` → carpeta pCloud `_PDFLector`,
   incluyendo PDFs y respaldos JSON, excluyendo temporales.
4. En ambos, desactivar propagación de borrados y movimiento de archivos.
   Activar transferencia mediante temporal y comprobar permisos y programación.
5. Configurar PDFLector mediante `files/folder_sync.json` de la aplicación.
   Campos: `enabled`, `incoming_root`, `outgoing_root`. Las dos raíces son
   absolutas y normalmente coinciden con la carpeta local de originales.
   El núcleo añade `_PDFLector` a `outgoing_root`; no incluirlo en ese campo.
   Ausencia o desactivación de la configuración deja el guardado local operativo.
6. Reiniciar PDFLector tras cambiar su configuración. No borrar sus datos.

La carpeta de entrada de la tablet es una copia de transporte. PDFLector guarda
sus originales de lectura y sidecars en su espacio privado. No sincronizar bases
SQLite abiertas ni el estado completo de biblioteca entre dispositivos.

## Comprobación

Utilizar un PDF sintético, sin datos personales. Confirmar descarga, aparición en
biblioteca, escritura, subrayado y publicación. Abrir la copia de `Apuntes` con el
lector del ordenador. Repetir una edición: no debe duplicar las anotaciones.
Comprobar desconexión/reconexión, reinicio con publicación pendiente, borrado en
origen y sustitución del original: la copia ya anotada debe conservarse.
La edición en otro lector del ordenador no se incorpora al modelo de PDFLector.
Registrar build y dispositivo en `docs/benchmark-results.md`, sin atribuir medidas
al flujo real a partir de pruebas sintéticas. No prometer latencia de nube ni
actividad en segundo plano sin comprobarlas en la TCL.
