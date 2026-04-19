#!/usr/bin/env bash
# Sanity-check each bench binary: start it, curl both routes, kill it.
set -euo pipefail
cd "$(dirname "$0")"

PORT=18081
for fw in flowgate axum actix hyper; do
  echo "-- $fw --"
  BENCH_WORKERS=1 BENCH_PORT="$PORT" "./target/release/$fw-bench" >/dev/null 2>&1 &
  pid=$!
  # Wait for port
  for _ in $(seq 1 50); do
    if (echo > "/dev/tcp/127.0.0.1/$PORT") 2>/dev/null; then break; fi
    sleep 0.1
  done
  printf "  GET  /plaintext → "
  curl -sS "http://127.0.0.1:$PORT/plaintext"; echo
  printf "  POST /echo      → "
  curl -sS -H 'content-type: application/json' \
    -d '{"name":"alice","id":42}' \
    "http://127.0.0.1:$PORT/echo"; echo
  kill "$pid" 2>/dev/null || true
  wait "$pid" 2>/dev/null || true
done
