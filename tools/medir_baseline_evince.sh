#!/usr/bin/env bash
# Baseline de rendimiento de Evince (backend poppler) para PDFLector.
# - Render time por página: pdftoppm usa exactamente el pipeline poppler+cairo
#   de Evince (poppler_page_render sobre cairo), single-thread.
# - RSS del visor GUI con PDF de 500 páginas (requiere display gráfico).
# - Medición sin GNU time (no instalado): wrapper python3 con resource.ru_maxrss.
# Uso: tools/medir_baseline_evince.sh [PDF]   (default: corpus/large_document.pdf)
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PDF="${1:-$ROOT/corpus/large_document.pdf}"
WORK="${TMPDIR:-/tmp}/evince-baseline"
mkdir -p "$WORK"

# Wrapper: mide wall time y max RSS (ru_maxrss, kB) de un comando
WRAP="$WORK/measure.py"
cat > "$WRAP" <<'PYEOF'
import resource, subprocess, sys, time
t0 = time.monotonic()
p = subprocess.run(sys.argv[1:], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
wall = time.monotonic() - t0
rss = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(f"wall {wall:.2f}s | maxRSS {rss} kB | exit {p.returncode}")
PYEOF

echo "PDF: $PDF ($(pdfinfo "$PDF" | awk '/Pages/{print $2}' | head -1) páginas)"
echo "HW: $(lscpu | awk -F': +' '/Model name/{print $2}'), $(free -h | awk '/Mem:/{print $2}') RAM"

run() {
	local label="$1"; shift
	echo "== $label =="
	for i in 1 2 3; do
		python3 "$WRAP" "$@"
	done
}

run "render 500 pág @72 dpi (1x)" pdftoppm -png -r 72 "$PDF" "$WORK/p72"
run "render 500 pág @144 dpi (2x)" pdftoppm -png -r 144 "$PDF" "$WORK/p144"
run "primera página @144 dpi (apertura+render)" pdftoppm -png -r 144 -f 1 -l 1 "$PDF" "$WORK/first"
run "primera página @216 dpi (3x)" pdftoppm -png -r 216 -f 1 -l 1 "$PDF" "$WORK/first216"

echo "== RSS Evince (GUI) con $PDF =="
measure_rss() { # $1: segundos de espera, $2: nota
	local sleep_s="$1" note="$2"
	setsid evince "$PDF" >/dev/null 2>&1 &
	local pid=$!
	sleep "$sleep_s"
	local rss=$(ps -o rss= -p "$pid" 2>/dev/null | tr -d ' ')
	echo "RSS ($note): ${rss:-?} kB"
	kill "$pid" 2>/dev/null
	sleep 1
	pkill -x evince 2>/dev/null
	sleep 1
}
measure_rss 8 "ventana abierta, página 1, tras 8 s"
measure_rss 8 "reabierto (caché de disco del sistema ya caliente), página 1"

echo "== limpieza =="
rm -f "$WORK"/p72*.png "$WORK"/p144*.png "$WORK"/first*.png "$WRAP"
echo "OK (PNG temporales borrados)"
