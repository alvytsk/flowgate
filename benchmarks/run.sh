#!/usr/bin/env bash
# Flowgate benchmark harness.
#
# Compares Flowgate against axum, actix-web, and raw hyper across:
#   modes:        single-thread (workers=1), multi-thread (workers=N)
#   workloads:    /plaintext (GET), /echo (POST 1KB JSON)
#   concurrency:  1, 64, 256
#   reps:         3 (median reported)
#
# Captures: RPS, latency p50/p95/p99, peak RSS, binary size.
# Writes a markdown report to results/results-YYYY-MM-DD.md.

set -euo pipefail

cd "$(dirname "$0")"

WORKERS_MULTI="${WORKERS_MULTI:-$(nproc)}"
DURATION="${DURATION:-30s}"
WARMUP="${WARMUP:-5s}"
REPS="${REPS:-3}"
PORT="${PORT:-18080}"
HOST="127.0.0.1"
RESULTS_DIR="results"
DATE="$(date +%F)"
REPORT="$RESULTS_DIR/results-$DATE.md"

FRAMEWORKS=("flowgate" "axum" "actix" "hyper")
MODES=("single" "multi")
WORKLOADS=("plaintext" "echo")
CONCURRENCY=(1 64 256)

ECHO_PAYLOAD='{"name":"benchmark-payload-of-modest-size-with-some-text-to-pad-near-1kb","id":4815162342,"_pad":"0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"}'
echo "$ECHO_PAYLOAD" > /tmp/flowgate-bench-echo.json

mkdir -p "$RESULTS_DIR"

# ----------------------------------------------------------------------------
# Build all benches in release with workspace profile (LTO=thin, cgu=1).
# ----------------------------------------------------------------------------
echo "==> Building all bench binaries (release, LTO=thin)..."
cargo build --release --quiet

bin_size() {
  # Strips path, returns size in bytes.
  stat -c '%s' "target/release/$1-bench"
}

# ----------------------------------------------------------------------------
# Sample peak RSS in KiB for a PID until it exits or we kill the sampler.
# ----------------------------------------------------------------------------
sample_rss() {
  local pid="$1" out="$2"
  local peak=0
  while kill -0 "$pid" 2>/dev/null; do
    local rss
    rss=$(awk '/^VmRSS:/ {print $2}' "/proc/$pid/status" 2>/dev/null || echo 0)
    if [ -n "$rss" ] && [ "$rss" -gt "$peak" ]; then peak="$rss"; fi
    sleep 0.1
  done
  echo "$peak" > "$out"
}

# ----------------------------------------------------------------------------
# Run oha against a target. Outputs JSON to stdout. Single rep.
# ----------------------------------------------------------------------------
run_oha() {
  local conc="$1" workload="$2"
  case "$workload" in
    plaintext)
      oha -z "$DURATION" -c "$conc" --no-tui --output-format json \
          "http://$HOST:$PORT/plaintext"
      ;;
    echo)
      oha -z "$DURATION" -c "$conc" --no-tui --output-format json \
          -m POST -T 'application/json' \
          -d "$(cat /tmp/flowgate-bench-echo.json)" \
          "http://$HOST:$PORT/echo"
      ;;
  esac
}

# ----------------------------------------------------------------------------
# Wait for port to accept connections (up to 5s).
# ----------------------------------------------------------------------------
wait_port() {
  for _ in $(seq 1 50); do
    if (echo > "/dev/tcp/$HOST/$PORT") 2>/dev/null; then return 0; fi
    sleep 0.1
  done
  return 1
}

# ----------------------------------------------------------------------------
# Run one (framework, mode, workload, concurrency) cell. Median of REPS reps.
# Writes a TSV line to $REPORT.tsv: framework mode workload conc rps p50 p95 p99 rss_kib
# ----------------------------------------------------------------------------
run_cell() {
  local fw="$1" mode="$2" workload="$3" conc="$4"
  local workers
  if [ "$mode" = "single" ]; then workers=1; else workers="$WORKERS_MULTI"; fi

  # Start server.
  local rss_file; rss_file="$(mktemp)"
  BENCH_WORKERS="$workers" BENCH_PORT="$PORT" \
    "./target/release/$fw-bench" >/dev/null 2>&1 &
  local pid="$!"
  trap 'kill $pid 2>/dev/null || true' RETURN

  if ! wait_port; then
    echo "  !! $fw $mode failed to bind in time" >&2
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
    echo -e "$fw\t$mode\t$workload\t$conc\tNA\tNA\tNA\tNA\tNA" >> "$REPORT.tsv"
    return
  fi

  # RSS sampler in background.
  sample_rss "$pid" "$rss_file" &
  local sampler="$!"

  # Warmup.
  oha -z "$WARMUP" -c "$conc" --no-tui --output-format json \
    "http://$HOST:$PORT/plaintext" >/dev/null 2>&1 || true

  # REPS measurement runs, take median by RPS.
  local rps_arr=() p50_arr=() p95_arr=() p99_arr=()
  for r in $(seq 1 "$REPS"); do
    local json; json="$(run_oha "$conc" "$workload" 2>/dev/null)"
    local rps p50 p95 p99
    rps=$(echo "$json" | jq -r '.summary.requestsPerSec // 0')
    p50=$(echo "$json" | jq -r '.latencyPercentiles.p50 // 0')
    p95=$(echo "$json" | jq -r '.latencyPercentiles.p95 // 0')
    p99=$(echo "$json" | jq -r '.latencyPercentiles.p99 // 0')
    rps_arr+=("$rps"); p50_arr+=("$p50"); p95_arr+=("$p95"); p99_arr+=("$p99")
  done

  # Median (sort ascending, pick middle).
  local med_rps med_p50 med_p95 med_p99
  med_rps=$(printf '%s\n' "${rps_arr[@]}" | sort -n | awk 'NR==int((NR+'"${#rps_arr[@]}"')/2+0.5){print; exit}')
  med_p50=$(printf '%s\n' "${p50_arr[@]}" | sort -n | awk 'NR==int((NR+'"${#p50_arr[@]}"')/2+0.5){print; exit}')
  med_p95=$(printf '%s\n' "${p95_arr[@]}" | sort -n | awk 'NR==int((NR+'"${#p95_arr[@]}"')/2+0.5){print; exit}')
  med_p99=$(printf '%s\n' "${p99_arr[@]}" | sort -n | awk 'NR==int((NR+'"${#p99_arr[@]}"')/2+0.5){print; exit}')

  # Stop server, wait for sampler.
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
  wait "$sampler" 2>/dev/null || true
  trap - RETURN

  local rss_kib; rss_kib=$(cat "$rss_file" 2>/dev/null || echo 0)
  rm -f "$rss_file"

  printf '  %-8s %-7s %-9s c=%-4s  RPS=%-10s  p50=%-7s p95=%-7s p99=%-7s  RSS=%sKiB\n' \
    "$fw" "$mode" "$workload" "$conc" "$med_rps" "${med_p50}s" "${med_p95}s" "${med_p99}s" "$rss_kib"

  echo -e "$fw\t$mode\t$workload\t$conc\t$med_rps\t$med_p50\t$med_p95\t$med_p99\t$rss_kib" >> "$REPORT.tsv"
}

# ----------------------------------------------------------------------------
# Header for the TSV.
# ----------------------------------------------------------------------------
: > "$REPORT.tsv"

# ----------------------------------------------------------------------------
# Run the matrix.
# ----------------------------------------------------------------------------
echo "==> Running matrix (workers: 1 / $WORKERS_MULTI; reps=$REPS; duration=$DURATION)..."
for fw in "${FRAMEWORKS[@]}"; do
  for mode in "${MODES[@]}"; do
    for workload in "${WORKLOADS[@]}"; do
      for c in "${CONCURRENCY[@]}"; do
        run_cell "$fw" "$mode" "$workload" "$c"
      done
    done
  done
done

# ----------------------------------------------------------------------------
# Render markdown report.
# ----------------------------------------------------------------------------
{
  echo "# Flowgate v0.2.0 benchmarks — $DATE"
  echo
  echo "Comparators: **flowgate**, **axum 0.7**, **actix-web 4**, raw **hyper 1.x**."
  echo
  echo "## Environment"
  echo
  echo "- Host: \`$(uname -srm)\`"
  echo "- CPU: \`$(awk -F: '/model name/ {print $2; exit}' /proc/cpuinfo | sed 's/^ *//')\`"
  echo "- Cores: \`$(nproc)\`"
  echo "- Workers (multi-thread mode): \`$WORKERS_MULTI\`"
  echo "- Tooling: oha \`$(oha --version | awk '{print $2}')\`, rustc \`$(rustc --version | awk '{print $2}')\`"
  echo "- Profile: \`release\` + \`lto = \"thin\"\`, \`codegen-units = 1\`, \`opt-level = 3\`"
  echo "- Each cell: $WARMUP warmup, $DURATION measurement, $REPS reps, **median** reported."
  echo
  echo "**Caveats:** WSL2 host (no kernel pinning, noisy neighbors). Treat results as **relative comparisons**, not absolute throughput numbers."
  echo
  echo "## Binary sizes (release, stripped of debuginfo by LTO)"
  echo
  echo "| Framework | Size |"
  echo "|---|---|"
  for fw in "${FRAMEWORKS[@]}"; do
    sz=$(bin_size "$fw")
    printf "| %s | %s KiB |\n" "$fw" "$((sz / 1024))"
  done
  echo
  for mode in "${MODES[@]}"; do
    label="single-thread (workers=1)"
    [ "$mode" = "multi" ] && label="multi-thread (workers=$WORKERS_MULTI)"
    echo "## $label"
    echo
    for workload in "${WORKLOADS[@]}"; do
      route="GET /plaintext"
      [ "$workload" = "echo" ] && route="POST /echo (1KB JSON in/out)"
      echo "### $route"
      echo
      echo "| Framework | Concurrency | RPS | p50 | p95 | p99 | Peak RSS |"
      echo "|---|---:|---:|---:|---:|---:|---:|"
      for fw in "${FRAMEWORKS[@]}"; do
        for c in "${CONCURRENCY[@]}"; do
          line=$(awk -F'\t' -v fw="$fw" -v m="$mode" -v w="$workload" -v c="$c" \
            '$1==fw && $2==m && $3==w && $4==c {print}' "$REPORT.tsv")
          if [ -z "$line" ]; then continue; fi
          rps=$(echo "$line" | awk -F'\t' '{print $5}')
          p50=$(echo "$line" | awk -F'\t' '{print $6}')
          p95=$(echo "$line" | awk -F'\t' '{print $7}')
          p99=$(echo "$line" | awk -F'\t' '{print $8}')
          rss=$(echo "$line" | awk -F'\t' '{print $9}')
          # Format numbers
          rps_fmt=$(printf '%.0f' "$rps" 2>/dev/null || echo "$rps")
          p50_ms=$(awk -v v="$p50" 'BEGIN{printf "%.2f ms", v*1000}')
          p95_ms=$(awk -v v="$p95" 'BEGIN{printf "%.2f ms", v*1000}')
          p99_ms=$(awk -v v="$p99" 'BEGIN{printf "%.2f ms", v*1000}')
          rss_mib=$(awk -v v="$rss" 'BEGIN{printf "%.1f MiB", v/1024}')
          printf "| %s | %s | %s | %s | %s | %s | %s |\n" \
            "$fw" "$c" "$rps_fmt" "$p50_ms" "$p95_ms" "$p99_ms" "$rss_mib"
        done
      done
      echo
    done
  done
  echo "## Raw data"
  echo
  echo "Per-cell TSV: \`$REPORT.tsv\` (columns: framework, mode, workload, concurrency, rps, p50_s, p95_s, p99_s, rss_kib)."
} > "$REPORT"

echo
echo "==> Report written to: $REPORT"
echo "==> TSV written to:   $REPORT.tsv"
