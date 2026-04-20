# Flowgate benchmarks

Comparative load test of Flowgate against three popular Rust HTTP servers:

| Crate | Version |
|---|---|
| [`flowgate`](../) | path dep (this repo) |
| [`axum`](https://crates.io/crates/axum) | 0.7 |
| [`actix-web`](https://crates.io/crates/actix-web) | 4 |
| [`hyper`](https://crates.io/crates/hyper) (raw) | 1.x |

## Workloads

Each binary exposes the same two routes:

- `GET /plaintext` → `"Hello, World!"`
- `POST /echo` → JSON in (`{"name": str, "id": int}` ~1 KB) → same JSON out

## Modes

- **single-thread** — `BENCH_WORKERS=1` (current-thread tokio runtime)
- **multi-thread** — `BENCH_WORKERS=$(nproc)` (multi-thread runtime)

The same binary handles both modes via the `BENCH_WORKERS` env var.

## Running

```bash
# install loadgen if needed
cargo install oha

# run the full matrix (4 frameworks × 2 modes × 2 workloads × 3 concurrency × 3 reps)
./run.sh

# customize:
DURATION=10s REPS=1 ./run.sh         # quick smoke run
WORKERS_MULTI=4 ./run.sh             # cap multi-thread workers at 4
PORT=19090 ./run.sh                  # use a different port
```

Results land in `results/results-YYYY-MM-DD.md` (markdown report) and the raw `.tsv` alongside it.

## What the harness does

1. Builds all four bench binaries in `release` profile (`lto = "thin"`, `codegen-units = 1`).
2. For each (framework, mode, workload, concurrency) cell:
   - Spawns the server with `BENCH_WORKERS` set.
   - Waits for the port to bind.
   - Background-samples `/proc/$pid/status` `VmRSS:` every 100ms (peak captured).
   - Runs `oha` warmup, then `REPS` measurement runs.
   - Reports the **median** RPS / p50 / p95 / p99 across reps.
   - Kills the server cleanly.
3. Renders a markdown report grouped by mode → workload → framework × concurrency.

## Caveats

- **WSL2 host.** No kernel-level pinning, noisy host. Results are useful as **relative comparisons**, not absolute throughput numbers.
- **Loopback only.** All traffic stays on `127.0.0.1`; no NIC overhead.
- **Same-machine loadgen.** `oha` competes with the server for CPU cycles. The single-thread mode is the most affected (loadgen can be heavier than the server).
- **Plaintext route is best-case** for every framework — minimal allocation, no parsing, no extractor work. The `/echo` route is closer to a realistic workload.
- **No keep-alive limits, no connection-count limits.** Frameworks use their stock defaults.
