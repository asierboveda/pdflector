# AGENTS.md — Constitución operativa de PDFLector

Este archivo define **cómo deben trabajar los agentes y las personas en este
repositorio**. No describe el estado completo del producto, no contiene un
roadmap y no funciona como backlog.

## 1. Orden obligatorio de lectura

Antes de analizar, diseñar o modificar el proyecto:

1. Leer este archivo completo.
2. Leer `ESTADO_ACTUAL.md` completo.
3. Leer el Issue asignado o, si no existe, el prompt explícito del propietario.
4. Inspeccionar el código y las pruebas del área afectada.
5. Abrir decisiones, evidencia o skills solo cuando sean relevantes para la
   tarea.

No usar documentos históricos como sustituto de inspeccionar el código. Si un
archivo contradice el comportamiento ejecutable, aplicar el protocolo de la
sección 4.

## 2. Norte del producto

PDFLector es una aplicación personal para leer y anotar PDFs en una tablet TCL
NXTPaper 11 Plus (9469X) con lápiz.

Prioridades, en este orden:

1. **Fluidez y respuesta inmediata.**
2. **Escritura y subrayado fiables.** El trazo debe sentirse natural y
   predecible; no basta con que los puntos terminen guardados correctamente.
3. **Una UX excelente para leer y tomar apuntes.**
4. **Uso razonable de memoria, CPU, GPU y batería**, sin sacrificar las tres
   prioridades anteriores.

El núcleo del producto es:

- abrir y leer PDFs;
- navegar y manipular el documento con fluidez;
- escribir con bolígrafo;
- subrayar texto.

Biblioteca, IA, Discover/arXiv, exportación y sincronización existen o pueden
existir, pero son capacidades periféricas. Pueden cambiarse o eliminarse sin
redefinir el producto.

La aplicación se diseña para uso personal e instalación directa. No asumir
requisitos de Play Store, multiusuario o servicio público salvo petición
explícita.

## 3. Fuentes de verdad

Cada tipo de información tiene un único destino:

| Información | Fuente canónica |
|---|---|
| Reglas de trabajo | `AGENTS.md` |
| Software que existe | `ESTADO_ACTUAL.md`, contrastado con el código |
| Trabajo pendiente | GitHub Issues |
| Decisión difícil de revertir | `docs/adr/ADR-*.md` |
| Medición obtenida | `docs/benchmark-results.md` |
| Procedimiento repetible | `.agents/skills/<nombre>/SKILL.md` |
| Presentación pública | `README.md` |

No crear roadmaps, fases, documentos de deuda, planes de implementación ni
listas paralelas de tareas dentro del repositorio. GitHub Issues es la única
cola persistente. El prompt selecciona el Issue; no reemplaza su contexto.

## 4. Protocolo ante contradicciones

No resolver contradicciones eligiendo el texto que parezca más convincente.
Investigar en este orden:

1. Punto de entrada y flujo de llamadas realmente conectado.
2. Pruebas que ejercitan ese flujo.
3. Manifiestos, scripts y CI que lo construyen.
4. Ejecución o medición en la TCL cuando intervengan Android, GPU, input o UX.
5. Comentarios y documentación.

Clasificar el resultado:

- **Verificado:** demostrado por prueba, build o medición reproducible.
- **Observado:** confirmado manualmente, indicando dispositivo y flujo.
- **Inferido:** deducido del código pero no ejecutado.
- **No verificado:** no existe evidencia suficiente.

Actualizar la fuente canónica afectada en el mismo cambio. No reescribir una
decisión histórica para fingir que nunca existió: sustituirla con otra decisión
cuando sea necesario.

## 5. Arquitectura de referencia

La arquitectura observable está descrita en `ESTADO_ACTUAL.md`. Estas reglas
son la referencia hasta que el propietario apruebe una sustitución:

- `pdf_android` es el producto y `pdf_app` es un cliente de escritorio/banco de
  pruebas, no la plataforma objetivo.
- `pdf_core` no tiene dependencias técnicas de frameworks de UI ni de Android.
  Los tipos conceptualmente visuales deben seguir siendo neutrales a la
  plataforma o salir del core.
- Las anotaciones se modelan en coordenadas de página y se pintan como una capa
  vectorial sobre el documento.
- La UI no espera render, red, persistencia ni exportación síncrona.
- Las cachés y colas tienen límites explícitos; no se retienen todas las páginas
  de un documento.
- El camino interactivo evita trabajo cuyo coste crezca sin control con el
  tamaño del PDF o con la duración de una sesión.
- Rust, Android nativo, EGL/GLES2 y MuPDF son el baseline actual, no dogmas. Un
  agente puede proponer alternativas con evidencia, pero no iniciar una
  migración sin aprobación.

Una propuesta de sustitución tecnológica debe comparar sobre el mismo hardware
y flujo: beneficio medido, coste de migración, riesgos, licencias y estrategia
de reversión.

## 6. Método de trabajo

### 6.1 Antes de editar

1. Confirmar objetivo, alcance y criterio observable de cierre.
2. Inspeccionar el estado de Git y preservar cambios ajenos.
3. Leer las rutas implicadas y sus consumidores; no diseñar desde nombres de
   fichero o comentarios aislados.
4. Buscar tests y mediciones existentes.
5. Clasificar el cambio por riesgo y aplicar las aprobaciones de la sección 7.

### 6.2 Durante el cambio

- Hacer el cambio mínimo que resuelve el objetivo aprobado.
- Mantener separadas lógica, presentación, persistencia y operaciones externas.
- Preferir interfaces pequeñas, estado explícito y componentes comprobables de
  forma aislada.
- No introducir abstracciones para futuros hipotéticos.
- No corregir problemas fuera de alcance. Crear o proponer un Issue cuando el
  hallazgo merezca trabajo posterior.
- Medir antes de optimizar y conservar únicamente cambios que mejoren el flujo
  objetivo sin degradar otro criterio esencial.

### 6.3 Después de editar

1. Revisar el diff completo.
2. Ejecutar la validación proporcional de la sección 10.
3. Actualizar decisión, evidencia, procedimiento o estado si el cambio altera
   su fuente canónica.
4. Informar qué cambió, qué se verificó y qué no pudo verificarse.
5. Pedir confirmación antes de crear un commit.

## 7. Aprobaciones obligatorias

Presentar diseño, alternativas relevantes y evidencia, y esperar aprobación
antes de modificar:

- UX o UI;
- arquitectura o límites entre componentes;
- dependencias nuevas o sustitución de tecnología;
- formatos persistentes y migraciones de datos;
- licencias o distribución;
- seguridad, permisos o gestión de claves;
- operaciones destructivas o difíciles de revertir.

La aprobación de una arquitectura se solicita **explicándola al propietario
antes de implementarla**. La explicación debe concretar el problema y el flujo
actual, los componentes y el recorrido de los datos propuestos, las
alternativas relevantes, por qué se recomienda una, sus efectos sobre la UX,
dependencias y datos existentes, y cómo se verificará y podrá revertirse.
Separar lo demostrado de lo que todavía es una hipótesis. Pedir el visto bueno
para ese diseño concreto, no una elección de tecnología sin contexto.

No construir una aplicación o APK experimental como requisito previo para
obtener ese visto bueno ni presentar un prototipo como sustituto de la
explicación. Un experimento aislado solo se hace cuando el propietario lo pide
o aprueba expresamente su objetivo y alcance. Una vez aprobado el diseño,
implementarlo en el producto dentro del alcance acordado, sin volver a pedir
la misma aprobación.

Correcciones triviales, mantenimiento mecánico y errores evidentes de bajo
riesgo pueden ejecutarse directamente si no cambian comportamiento ni contrato.

La aprobación de un diseño autoriza la implementación descrita, no autoriza el
commit, una publicación, un borrado ni trabajo adicional.

## 8. UX y cambios visuales

Todo cambio de UX/UI requiere:

1. describir el problema del usuario y el flujo afectado;
2. presentar alternativas y recomendar una;
3. obtener aprobación antes de editar;
4. comprobar interacción, estados vacíos, errores, orientación y accesibilidad;
5. validar en la TCL, no solo por inspección de código.

No confundir "está implementado" con "se siente bien". Escritura, selección,
scroll, zoom y navegación se evalúan con uso real además de métricas.

Las ideas futuras pertenecen a Issues. No describir una interfaz planificada en
`ESTADO_ACTUAL.md` como si estuviera disponible.

## 9. Rendimiento

La pantalla de referencia admite 60 y 120 Hz a 1440×2200. PDFLector debe
solicitar 120 Hz mientras su superficie esté activa y funcionar correctamente
si Android mantiene 60 Hz por política, batería o temperatura. Registrar el
refresco efectivo en cada medición.

Presupuestos iniciales para la TCL 9469X:

| Flujo | Objetivo |
|---|---|
| Escritura, selección, pinch y manipulación directa | trabajo de app p95 ≤ 6 ms |
| Presentación interactiva a 120 Hz | frame p95 ≤ 8,33 ms; p99 ≤ 16,67 ms |
| Frames perdidos durante interacción continua | < 1 % |
| Evento de stylus → envío de frame | p95 ≤ 8,33 ms |
| Lápiz → píxel, medido externamente | p95 ≤ 25 ms |
| Respuesta visual al cambio de página | ≤ 16,67 ms |
| Página cacheada visible | ≤ 50 ms |
| Página no cacheada nítida | ≤ 200 ms, sin bloquear input |
| PSS estable | ≤ 250 MB |
| Pico de estrés | ≤ 350 MB y sin crecimiento monotónico entre ciclos |

Estos son presupuestos de ingeniería, no afirmaciones sobre el estado actual.
La primera medición reproducible puede ajustarlos si demuestra un límite físico
o del sistema. No relajarlos ni endurecerlos sin registrar evidencia y obtener
aprobación.

Detener una optimización cuando el flujo cumpla sus percentiles en varios
ensayos, con temperatura estable, y no exista un defecto perceptible. No seguir
iterando por una mejora numérica sin impacto observable.

Cada medición debe incluir como mínimo:

- fecha y commit;
- dispositivo, versión de Android y build instalada;
- resolución y refresco efectivo;
- estado térmico inicial/final;
- PDF o corpus y pasos exactos;
- métrica, percentiles, número de muestras y resultado bruto;
- comparación antes/después cuando evalúe un cambio.

## 10. Validación

Aplicar el carril correspondiente; una comprobación de compilación no sustituye
una prueba de dispositivo.

### Cambios en `pdf_core`

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test -p pdf_core
```

### Cambios en el cliente de escritorio

```bash
cargo check -p pdf_app
```

Ejecutar pruebas específicas adicionales si se modifica su comportamiento.

### Cambios Android

```bash
export ANDROID_NDK_HOME=/home/asierboveda/Android/Sdk/ndk/android-ndk-r28
export PATH="$HOME/.cargo/bin:$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/sysroot"
cargo check -p pdf_android --target aarch64-linux-android --all-targets
cargo clippy -p pdf_android --target aarch64-linux-android --all-targets -- -D warnings
```

El agente puede compilar, instalar una APK de prueba y ejecutar ADB como parte
normal de una tarea Android. Debe anunciarlo antes. Puede observar, capturar y
medir; no puede borrar datos personales, cambiar ajustes persistentes ni
eliminar PDFs sin aprobación explícita.

Los cambios en render, input, tinta, selección, zoom, caché o lifecycle deben
probarse en la TCL. Registrar como no verificado cualquier flujo que no se haya
podido ejecutar allí.

### Cambios de documentación

- comprobar enlaces y rutas;
- buscar referencias a documentos eliminados;
- ejecutar `git diff --check`;
- contrastar afirmaciones técnicas con código o evidencia.

## 11. Trabajo con varios agentes

El agente principal es el orquestador: descompone, asigna, reconcilia y valida
el resultado integrado. Cuando la plataforma lo permita, usar agentes Luna para
trabajo paralelo.

- Paralelizar solo dominios realmente independientes.
- Dar a cada agente objetivo, rutas, límites y resultado verificable.
- Los agentes de auditoría trabajan en solo lectura.
- Dos agentes no editan el mismo fichero ni estado compartido.
- Los agentes que editan usan ámbitos de ficheros disjuntos o worktrees
  separados.
- El orquestador revisa los informes y ejecuta la verificación integrada; no
  acepta una conclusión únicamente porque la produjo un subagente.
- Una sola persona o agente integra cambios en una rama cada vez.

No lanzar agentes para aparentar paralelismo cuando el trabajo es secuencial o
la coordinación cuesta más que la tarea.

## 12. Seguridad y datos

- Nunca introducir claves, tokens, credenciales, PDFs personales ni datos del
  usuario en Git, logs persistentes o artefactos compartidos.
- Las claves locales de IA pueden incorporarse a una build personal, pero sus
  ficheros deben permanecer ignorados por Git. Usar placeholders para CI.
- Rutas de keystore y credenciales de firma deben vivir en configuración local,
  no en manifiestos versionados. Una configuración heredada no es precedente
  para cambios nuevos.
- No imprimir el contenido de claves durante diagnóstico.
- La aplicación y sus herramientas nunca borran automáticamente un PDF del
  usuario.
- Tratar intents, nombres de fichero, respuestas de red y contenido PDF como
  entradas no confiables.
- Revisar licencia y compatibilidad antes de añadir una dependencia.

## 13. Política de documentación

Actualizar documentación solo cuando tenga un consumidor futuro claro:

- cambio observable del software → `ESTADO_ACTUAL.md`;
- decisión aprobada y costosa de revertir → ADR;
- medición → evidencia;
- procedimiento repetible → skill;
- tarea pendiente → Issue;
- cambio de reglas de trabajo → `AGENTS.md`;
- cambio de uso público → `README.md`.

No duplicar la misma afirmación en varios archivos. Enlazar a la fuente
canónica. El historial de Git conserva lo eliminado; no mantener documentación
zombi con banners de "histórico".

## 14. Git, idioma y entrega

- Preservar cambios no relacionados que ya existan en el worktree.
- No usar comandos destructivos de Git para limpiar trabajo ajeno.
- No crear commits, tags, ramas remotas ni PR sin autorización explícita.
- Antes de solicitar el commit, mostrar el alcance, las verificaciones y todo lo
  que permanezca sin comprobar.
- Documentación y comunicación: español.
- Código, identificadores y mensajes de error técnicos: inglés.
- Mensajes de commit de código: inglés. Los commits solo documentales pueden
  escribirse en español.

## 15. Definición de hecho

Una tarea está terminada únicamente cuando:

1. cumple el objetivo y no amplía el alcance sin permiso;
2. pasa la validación aplicable;
3. las afirmaciones de rendimiento están medidas en el flujo y hardware
   declarados;
4. el comportamiento en tablet está verificado o marcado explícitamente como
   pendiente;
5. las fuentes canónicas afectadas están actualizadas;
6. el diff fue revisado y no contiene secretos ni cambios ajenos;
7. el propietario recibió un resumen y decidió si autoriza el commit.
