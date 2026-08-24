# BUG — La pantalla se apaga/parpadea al escribir (SIGUE SIN RESOLVER)

> **Sesión**: 2026-08-24 (tarde). **Estado**: ABIERTO — las hipótesis aplicadas
> no eliminan el síntoma en hardware real (lápiz USI). Este documento es el
> plan de la PRÓXIMA sesión.
> **Síntoma exacto (usuario)**: al escribir un trazo que **cruza tinta ya
> existente** y **soltar**, la pantalla "se apaga como un parpadeo"; a veces
> dura más. No es el timeout normal (ocurre en el instante del gesto).

---

## 1. Qué está PROBADO que NO es (historial de intentos)

| # | Hipotisis | Acción realizada | Resultado |
|---|-----------|------------------|-----------|
| 1 | Timeout del sistema (screen_off) | `FLAG_KEEP_SCREEN_ON` vía JNI en el arranque (jni.rs keep_screen_on) | **Aplicado y verificado**: `dumpsys window` → `fl=KEEP_SCREEN_ON`; reposo 40s + actividad 70s con `screen_off_timeout=10s` → `mWakefulness=Awake`. El timeout NO es la causa. |
| 2 | Re-aplicar el flag en InitWindow | Prueba en `lib.rs` InitWindow | **Descartado**: lanza `Only the original thread that created a view hierarchy...` (getWindow().addFlags desde hilo android_main); la llamada del arranque persiste. |
| 3 | Wakelock de pantalla como refuerzo | `PowerManager.newWakeLock(SCREEN_BRIGHT...)` | **Imposible en Android 15**: `Must specify a valid wake lock level` (niveles de pantalla eliminados en API 28+; solo PARTIAL/DOZE/PROXIMITY). Vía cerrada. |
| 4 | Sobrecarga al escribir encima de muchos trazos | Bug real encontrado y FIX: `end_tool_gesture` hacía doble-take de `gesture_base` → invalidaba el frame → recomponía las 276 anotaciones por trazo | **FIX aplicado** (6db8c1f). Verificado por adb (4 trazos cruzando 276 anotaciones, blits 3.8-5ms estables). |
| 5 | Palma/mano al soltar convierte el gesto en pinch (reescala = flash) | FIX: palm rejection — si hay ToolDrawing + stylus, el segundo puntero se ignora (77f6ef5) | **FIX aplicado** pero **NO verificable por adb** (no se puede inyectar stylus físico): pendiente de confirmación real. |
| 6 | Crash de la app (panic/SIGSEGV) | Reproducción por adb con dibujo forzado de dedo cruzando trazos + `dropbox` crash reports | **Sin crash**: no hay FATAL/SIGSEGV/tombstone; los únicos crash de dropbox son de builds viejas (setBuffersCount). La app sigue viva tras los gestos. |

**Datos del sistema observados (sin explicar)**:
- `surfaceflinger` loguea periódicamente `== MALI DEBUG === eglp_winsys_populate_image_templates ==12288` (driver Mali del compositor, cada ~3s).
- El proceso queda `S (sleeping)` 0% CPU en reposo (correcto).
- `RSS` de la app estable ~100-180MB (PSS ~101-133MB).

## 2. Por qué no podemos cerrarlo desde el escritorio

El síntoma SOLO se reproduce con el **lápiz físico** (USI) en la tablet:
`adb` no puede inyectar eventos de stylus (no hay `input stylus`), y el dedo
forzado no replica el parpadeo. Falta captura de logs EN EL MOMENTO del fallo
con el hardware real.

## 3. Hipótesis pendientes (orden de prioridad)

### H1 — La Activity se RECREA al soltar (parpadeo = ventana destruida/creada)
- El parpadeo completo + "a veces dura más" es el patrón de una
  `destroy → create` de la Activity (la app se relanza; el flag de la ventana
  se pierde en el hueco).
- Disparadores posibles en la TCL: rotación automática (sensor activo),
  `enter_immersive`/insets que reconfiguran, o política de la ROM al detectar
  input de stylus cruzando áreas ocupadas.
- **Cómo validar**: capturar logcat del ActivityManager en vivo
  (`ActivityTaskManager` / `am_*`) + `dumpsys activity top` justo después del
  gesto. Buscar `Displayed com.pdflector.app`, `DestroyActivity`, `onStop`.
- **Mitigación candidata**: declarar `android:configChanges` (orientation |
  screenSize | keyboardHidden) en el manifest para que la rotación NO recreé la
  Activity (cargo-apk: metadatos de la actividad — verificar si lo soporta;
  si no, release con manifest propio o `xbuild`).

### H2 — El compositor (SurfaceFlinger/driver Mali) hace un reset visual
- El `eglp_winsys_populate_image_templates` de SF es un debug del driver MALI;
  un device-lost/restart del compositor produce un flash negro global.
- **Cómo validar**: logcat de `surfaceflinger` alrededor del gesto + comparar
  timestamps con el parpadeo. Probar presentar el buffer en RGB565 (guard
  format) o reducir la frecuencia de presenta (ya coalescimos a 60Hz).

### H3 — El watchdog/hang de la ROM TCL (NVR) apaga/congela el display
- La TCL tiene un detector `nvr.BaseNvrHung` (visto en logs del sistema). Si el
  input dispatch de la app va lento (recomposición, save SQLite pesado en el
  hilo UI), la ROM podría reaccionar.
- **Cómo validar**: mover `save_annotations` (SQLite JSON de 276+ trazos) a un
  hilo de fondo (mpsc, como el worker) y ver si el parpadeo desaparece; buscar
  logs `nvr.*` en el momento.
- **Acción preventiva YA**: `save_annotations` bloquea el UI ~50-200ms con
  sidecar grande → mover a hilo (deferred).

### H4 — Política de batería/dim de la ROM con stylus (ignora KEEP_SCREEN_ON)
- Algunas ROMs bajan la pantalla con entrada de stylus aunque la app tenga el
  flag (patrones de "inactividad" por source).
- **Cómo validar**: en el momento del fallo (con adb), `dumpsys power | grep -E
  'mWakefulness|mScreenBrightness' + mHoldingWakeLockSuspendBlocker`; y el
  `settings get system screen_off_timeout` real.
- **Workaround si se confirma**: refresco periódico de "actividad" no es
  posible sin permisos; quedan: probar `svc power stayon true` como test de
  confirmación (si con stayon NO parpadea, es política del ROM que ignora el
  flag con stylus → decidir si aceptarlo o usar surface de video/VR).

## 4. Plan de acción (próxima sesión, en orden)

### Paso 0 — Preparar la captura en vivo (10 min)
1. Conectar la tablet por USB; `adb shell svc power stayon false`.
2. Script `tools/capture-screenoff.sh`:
   ```bash
   adb logcat -c
   adb logcat -v threadtime > /tmp/screenoff-$(date +%H%M%S).log &
   # ... disparador manual: reproducir el gesto (trazo cruzando tinta + soltar)
   adb shell dumpsys power > /tmp/power-before.txt
   # (después del gesto)
   adb shell dumpsys power > /tmp/power-after.txt
   adb shell dumpsys activity top | grep -A2 ACTIVITY > /tmp/activity.txt
   ```
3. Pedir al usuario que reproduzca el gesto con el lápiz (3-5 veces).

### Paso 1 — Analizar la captura (30 min)
- ¿`mWakefulness=Asleep` tras el gesto? → H4 (política ROM).
- ¿`ActivityManager: Destroyed Activity ... com.pdflector.app` o `Displayed` +
  `START`? → H1 → aplicar configChanges o frenar rotación
  (`settings put system accelerometer_rotation 0` como defensa temporal).
- ¿`surfaceflinger` errores MALI/`egl` en el instante? → H2 → prueba RGB565 /
  menos presents.
- ¿`nvr.*Hung` o `InputDispatcher` timeouts? → H3 → mover `save_annotations` a
  hilo + revisar bloqueos del UI.
- ¿Nada de lo anterior? → volver a H1 con `dumpsys window windows` (comprobar
  si el flag sobrevive al gesto: si desaparece `fl=KEEP_SCREEN_ON`, la ventana
  se recreó).

### Paso 2 — Aplicar la mitigación según el hallazgo (media sesión)
- **H1**: `configChanges` en cargo-apk (investigar soporte) o rotación fija.
- **H2**: probar `guard.format()` RGB565 (los blits ya soportan bpp=2) o
  limitar a un present por vsync con `dirty` (ya).
- **H3**: `save_annotations` → hilo de fondo (reutilizar patrón mpsc del
  worker/ai). Verificar que el UI no tiene ningún bloqueo >50ms en end_gesture
  (medir con log de fase).
- **H4**: si se confirma política de ROM con stylus: probar flag en la VENTANA
  TODAVÍA no basta → opciones: (a) aceptar y documentar; (b) mantener un
  "touch virtual" periódico es inviable; (c) cambiar a render EGL (Fase 6) —
  la vía que además soluciona el resto de latencia.

### Paso 3 — Fijación y cierre (resto de sesión)
- Con la mitigación activa, pedir al usuario 10 gestos seguidos cruzando tinta
  + soltar, y 5 minutos de escritura continua: criterio de cierre = 0
  apagados/parpadeos.
- Actualizar este documento con el hallazgo y el fix; referenciar el ADR si
  cambia arquitectura (p. ej. decisión de EGL).

## 5. Cosas que NO tocar durante el diagnóstico (para no liar)

- El flujo de tinta directa (stamping incremental) — verificado y correcto.
- El worker de render async — verificado estable.
- El flag KEEP_SCREEN_ON en el arranque — funciona y debe mantenerse.
- El palm rejection — correcto, mantener; confirmar con el usuario.

## 6. Referencias útiles

- `crates/pdf_android/src/jni.rs` → keep_screen_on (flag + historia del wakelock).
- `crates/pdf_android/src/lib.rs` → arranque, bucle con timeout 16ms.
- `crates/pdf_android/src/reader.rs` → end_tool_gesture / palm / worker / save.
- `crates/pdf_android/src/input.rs` → PointerDown + palm rejection + stylus.
- Logs de interés en vivo: `adb logcat -v threadtime | grep -E
  'ActivityTaskManager|InputDispatcher|surfaceflinger|nvr|pdf_android'`.