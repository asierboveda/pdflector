#!/usr/bin/env bash
# bench_evince.sh — baseline de rendimiento de Evince en escritorio.
#
# Mide:
#   - tiempo de arranque (cold) hasta que aparece la ventana en Hyprland
#   - RSS peak durante los primeros N segundos
#   - tiempo de carga de página específica (--page-index=N)
#
# NO inyecta keystrokes (no requiere hyprctl dispatch); para frame time p95
# ver docs/benchmarks/evince-baseline.md sección "Medición manual".
#
# Uso:
#   ./tools/bench-evince/bench_evince.sh <ruta-evince-binario> <directorio-corpus> [N segundos]
#
# Ejemplo:
#   ./tools/bench-evince/bench_evince.sh \
#       /tmp/opencode/evince-full/build/shell/evince \
#       ./corpus 25
#
# Salida: por cada PDF del corpus imprime una línea CSV-like y la guarda en
# <evince-binario>.results.csv

set -euo pipefail

EVINCE_BIN="${1:?Falta ruta al binario de Evince}"
CORPUS_DIR="${2:?Falta directorio del corpus}"
DURATION_S="${3:-25}"

if [[ ! -x "$EVINCE_BIN" ]]; then
    echo "ERROR: $EVINCE_BIN no existe o no es ejecutable" >&2
    exit 1
fi
if [[ ! -d "$CORPUS_DIR" ]]; then
    echo "ERROR: $CORPUS_DIR no existe" >&2
    exit 1
fi

# Nombre del binario (para el CSV).
EV_BASENAME=$(basename "$EVINCE_BIN")
RESULTS="$EVINCE_BIN.results.csv"

# Cabecera CSV.
echo "pdf,pages,size_kb,cold_start_ms,rss_peak_mb,rss_baseline_mb,page_jump_ms" > "$RESULTS"

# Detector de "ventana lista": aparece en `hyprctl clients` con la clase
# correcta. Evince usa "org.gnome.Evince" como app id.
ready_clients() {
    hyprctl clients -j 2>/dev/null \
        | python3 -c "import json,sys
data=json.load(sys.stdin)
for c in data:
    if c.get('class','').startswith('org.gnome.Evince') or 'evince' in c.get('class','').lower():
        print(c.get('class',''))
        sys.exit(0)
sys.exit(1)"
}

# Captura RSS en KiB de un PID. Devuelve 0 si el proceso sigue vivo.
rss_kib() {
    local pid=$1
    [[ -d /proc/$pid ]] || return 1
    awk '/VmRSS:/ { print $2 }' /proc/$pid/status 2>/dev/null
}

# Mata todos los procesos del árbol de un PID.
kill_tree() {
    local pid=$1
    local ppid
    for child in $(pgrep -P "$pid" 2>/dev/null || true); do
        kill_tree "$child"
    done
    kill -TERM "$pid" 2>/dev/null || true
}

run_one() {
    local pdf=$1
    local pages
    pages=$(uv run --with pypdf python3 -c "from pypdf import PdfReader; import sys; print(len(PdfReader(sys.argv[1]).pages))" "$pdf" 2>/dev/null) || pages=0
    local size_kb
    size_kb=$(($(stat -c %s "$pdf") / 1024))
    local label
    label=$(basename "$pdf")

    echo "== $label ($pages páginas, ${size_kb} KiB) ==" >&2

    # --- 1) Cold start (sin caché de página) ---------------------------
    # Lanzar Evince en su propia sesión, guardar PID, registrar RSS.
    setsid "$EVINCE_BIN" "$pdf" </dev/null >/tmp/bench-evince.log 2>&1 &
    local pid=$!
    local start_ns=$(date +%s%N)

    # Esperar a que aparezca la ventana en Hyprland (= "lista").
    local ready_ms=""
    while :; do
        if ready_clients >/dev/null; then
            local now_ns=$(date +%s%N)
            ready_ms=$(( (now_ns - start_ns) / 1000000 ))
            break
        fi
        # Timeout duro a 15s (PDF enorme).
        local now_ns=$(date +%s%N)
        if (( (now_ns - start_ns) > 15000000000 )); then
            ready_ms="TIMEOUT"
            break
        fi
        sleep 0.05
    done
    echo "  cold_start_ms=$ready_ms" >&2

    # Muestrear RSS durante $DURATION_S (incluye la carga inicial + idle).
    local rss_max_kib=0
    local rss_first_kib=0
    local end_ns=$(( $(date +%s%N) + DURATION_S * 1000000000 ))
    while (( $(date +%s%N) < end_ns )); do
        local rss
        rss=$(rss_kib "$pid") || break
        (( rss > rss_max_kib )) && rss_max_kib=$rss
        (( rss_first_kib == 0 )) && rss_first_kib=$rss
        sleep 0.2
    done
    local rss_peak_mb=$(( rss_max_kib / 1024 ))
    local rss_baseline_mb=$(( rss_first_kib / 1024 ))
    echo "  rss_baseline=${rss_baseline_mb} MiB  rss_peak=${rss_peak_mb} MiB" >&2

    # Limpiar antes del segundo test. Con setsid, el pgid de nuestra instancia
    # es su propio pid: kill -- -$pid mata solo nuestro árbol (evita pkill -f
    # global, que también mataría instancias de Evince del usuario).
    kill_tree "$pid" 2>/dev/null || true
    sleep 0.5
    kill -- -"$pid" 2>/dev/null || true
    sleep 0.5

    # --- 2) Salto a página lejana (cold cache de páginas) --------------
    # Cerrar y reabrir apuntando a página 75% del documento.
    local jump_ms=""
    if [[ "$pages" -gt 5 ]]; then
        local target=$(( pages * 3 / 4 ))
        setsid "$EVINCE_BIN" --page-index="$target" "$pdf" </dev/null >>/tmp/bench-evince.log 2>&1 &
        local pid2=$!
        local start2_ns=$(date +%s%N)
        while :; do
            if ready_clients >/dev/null; then
                local now2_ns=$(date +%s%N)
                jump_ms=$(( (now2_ns - start2_ns) / 1000000 ))
                break
            fi
            local now2_ns=$(date +%s%N)
            if (( (now2_ns - start2_ns) > 20000000000 )); then
                jump_ms="TIMEOUT"
                break
            fi
            sleep 0.05
        done
        echo "  page_jump_ms(${target}/${pages})=$jump_ms" >&2
        kill_tree "$pid2" 2>/dev/null || true
        sleep 0.5
        kill -- -"$pid2" 2>/dev/null || true
    else
        jump_ms="N/A"
    fi

    echo "$label,$pages,$size_kb,$ready_ms,$rss_peak_mb,$rss_baseline_mb,$jump_ms" >> "$RESULTS"
}

shopt -s nullglob
for pdf in "$CORPUS_DIR"/*.pdf; do
    run_one "$pdf"
done

echo
echo "Resultados guardados en: $RESULTS"
column -t -s, "$RESULTS"