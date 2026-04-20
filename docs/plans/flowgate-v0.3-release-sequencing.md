# Flowgate Release Sequencing: 0.2.0 → 0.3.0 → 0.3.1

## Context

Current `feature/v0.2` is architecturally complete: WebSocket, SSE, TLS, route groups, OpenAPI 3.1, pre-routing middleware, RequestId/Timeout/Recover, metrics observer, graceful shutdown, Path extractor — all shipped. CHANGELOG.md (2026-04-19) and a perf baseline (`benchmarks/results/results-2026-04-19.md`, `docs/perf-baseline.md`) already exist. What is missing for a credible first public release is one user-visible capability (static files) and an honest answer to the benchmark overhead the perf report surfaced.

Publishing 0.2.0 now would make the first public Flowgate feel transitional — users would immediately hit the missing static serving and the perf footnotes. The cleaner narrative:

- **Internal 0.2.0** — documented architectural checkpoint, **unpublished**. Freezes the foundation-expansion cycle.
- **Public 0.3.0** — static files land. First "whole framework" release. Surface expansion.
- **Public 0.3.1** — measured zero-cost-path cleanup on a frozen API shape. Perf-only story.

Splitting surface expansion from hot-path flattening keeps each release's regression story interpretable. The version jump from 0.1.x → 0.3.0 on crates.io is fine for pre-1.0 as long as the CHANGELOG is explicit.

The discipline to hold: 0.3.0 is **not** an open-ended perf campaign. Trivially obvious zero-cost fixes that surface naturally during static-files work are allowed in 0.3.0; systematic perf work is reserved for 0.3.1.

Branch mechanics (PR for 0.2.0 → main, fresh branch for static-files work) are handled by the user manually and are out of scope for this plan.

---

## Phase 1 — Internal 0.2.0 (unpublished checkpoint)

### Task 1.1 — Retrospective note for the internal milestone

**Summary:** Add `docs/releases/0.2.0-internal.md` capturing scope, perf baseline, deferrals, and perf hypotheses.

**Detail:** The CHANGELOG is a user-facing feature log. The retrospective is a builder-facing snapshot — what 0.2.0 *means* as an architectural moment. Content:

- **Scope frozen** — enumerate shipped surfaces: routing + groups, handler erasure model, pre- and post-routing middleware, extractors including `Path<T>` / `State<T>` / `Json<T>` / `WebSocketUpgrade`, `Sse<S>`, TLS, OpenAPI + Scalar UI, metrics observer, graceful shutdown, body-read timeout.
- **Perf baseline** — link to `benchmarks/results/results-2026-04-19.md` and `docs/perf-baseline.md`. Quote the headline numbers (alloc ceiling ≤26/req, `openapi_json_large` ~15% faster than v0.1). Note the baseline is the pre-0.3.1 reference.
- **Explicit deferrals to 0.3.0** — static file serving (was hedged to "0.2.1 or 0.2.2" in CHANGELOG line 182; now promoted to 0.3.0).
- **Known perf hypotheses for 0.3.1** — unconditional `RouteParams` allocation at `src/router.rs:94-100`; no zero-middleware fast path at `src/server.rs:384-389` / `src/middleware/mod.rs:72-84`; unconditional `boxed_unsync()` at `src/body.rs:28-37`; per-connection `TokioTimer` at `src/server.rs:157-158`. These are leads, not commitments.
- **Not published to crates.io** — state explicitly so the doc is its own evidence that the version exists only internally.

**Why a doc and not just a tag:** a tag is a pointer; a retrospective is load-bearing context when `git log` becomes noisy six months from now. Short is fine — half a page.

### Task 1.2 — Verify architecture doc matches shipped state

**Summary:** Re-read `docs/architecture.md` against the current code and fix any drift.

**Detail:** `docs/architecture.md` is the project's canonical design reference (per CLAUDE.md: "Always update it when architecture changes"). Spot-check the sections most likely to have drifted since 0.1:

- Layer diagram — does it show pre-routing middleware, WS upgrade branch, TLS carve-out?
- Handler erasure — still accurate for 0-8 extractor arms?
- Ownership boundary (`Arc<S>` in erased layer, `&S` in extractor layer) — unchanged, but verify.
- Middleware chain — mention `Arc<[Arc<dyn Middleware<S>>]>` and both `Next` / `PreNext` walkers.
- Builder/runtime split — `finalize()` flow intact.

Fix deltas inline. If a section is materially out of date (> one paragraph), note it in the retrospective doc too.

---

## Phase 2 — Public 0.3.0 (static files, full scope)

Static files are the sole surface expansion. User selected **full scope**: path-traversal safety, MIME detection, HEAD, index file, conditional GET (ETag + If-Modified-Since), Range requests (206 Partial Content), and SPA fallback. This is a non-trivial subsystem. It warrants its own dedicated sub-plan written with `superpowers:writing-plans` when the static-files branch starts.

### Task 2.1 — API design: shape of `ServeDir` / `ServeFile`

**Summary:** Decide the public API before writing code — responder vs endpoint vs middleware, config knobs, error surface.

**Detail:** Three shape decisions drive everything downstream:

1. **Integration point.** Candidates: (a) an `Endpoint<S>` impl registered via `.route("/assets/*path", ServeDir::new("./public"))`; (b) a `Responder` consumed by a user handler; (c) a fallback-style `App::fallback(ServeDir::...)`. Recommendation: **(a)** — fits the existing router/handler model, reuses the wildcard `*path` matchit syntax, and the fallback story (for SPA) is a separate flag on the same type.
2. **Config surface.** Fields: `root: PathBuf`, `index_file: Option<String>` (default `Some("index.html")`), `fallback: Option<PathBuf>` (enables SPA mode — serve this file on 404), `precompressed: bool` (off for 0.3.0 — add later if users ask), `cache_control: CacheControlPolicy` (enum: `NoStore` / `MaxAge(u32)` / `Immutable`). Keep the initial surface narrow.
3. **Error taxonomy.** `StaticError` with `NotFound` (404), `Forbidden` (403 — symlink escape / traversal), `IoError` (500), `PreconditionFailed` (412), `NotModified` (304 — not an error, but flows through the same `IntoResponse` bridge), `RangeNotSatisfiable` (416). Implements `IntoResponse`.

Write this as a short design doc (not a plan yet) — one page, for the user to sanction before the detailed TDD plan.

### Task 2.2 — Write the static-files sub-plan

**Summary:** Produce a step-by-step TDD plan at `docs/superpowers/plans/YYYY-MM-DD-static-files.md` using `superpowers:writing-plans`.

**Detail:** This plan — the one you're reading — is a roadmap, not an implementation plan. Static files need their own because the surface is large enough that bite-sized TDD tasks (failing test → minimal impl → pass → commit) are the right granularity. Expected task shape, in order:

- Path resolution + traversal defense (`..`, absolute paths, symlink escape). Write adversarial tests first.
- MIME detection. Candidate crates: `mime_guess` (ubiquitous, table-based) vs hand-rolled extension table. Choose `mime_guess` unless binary-size audit says otherwise.
- HEAD handling — share the GET code path, strip body.
- Index file resolution — when request path ends `/`, append `index_file`.
- Conditional GET — `If-None-Match` (ETag), `If-Modified-Since` (Last-Modified). ETag strategy: weak ETag from `(mtime, size)` is sufficient for 0.3.0.
- Range requests — single range first (`bytes=0-99`), multipart ranges deferred to 0.3.2 unless trivial. 416 for unsatisfiable.
- SPA fallback — when path 404s and `fallback` is configured, serve the fallback file with 200.
- Cache-Control header emission per policy.
- Integration tests against a real temp dir (use `tempfile` crate).
- `examples/static_files.rs` with both plain-dir and SPA-mode variants.

### Task 2.3 — Execute the sub-plan

**Summary:** Run the static-files sub-plan (subagent-driven or inline — user's choice at that time).

**Detail:** Pure execution. No new decisions at this layer. Discipline: no scope creep into perf work except for the one carve-out in 2.4.

### Task 2.4 — Opportunistic zero-cost-path fix (permitted, bounded)

**Summary:** If — and only if — the unconditional `RouteParams` allocation becomes genuinely noisy while implementing static files (wildcard `*path` routes always allocate params), fix it in 0.3.0.

**Detail:** The user's explicit carve-out: "If, while adding static files, you naturally see something obviously wrong like unconditional params allocation, it is okay to fix it right then. Just do not turn 0.3.0 into an open-ended perf campaign." The specific fix: at `src/router.rs:94-100`, guard the `RouteParams` collection behind `matched.params.is_empty()`, and only insert the extension when there is something to carry. Update any extractor that assumes the extension is present (`Path<T>` in `src/extract/`). If this pulls in more than ~30 lines of changes or touches extractor semantics, **stop and defer to 0.3.1** — the discipline is load-bearing.

Forbidden in 0.3.0 under any pretext: body un-boxing, middleware fast path, timer gating. Those ship in 0.3.1.

### Task 2.5 — Documentation, example, CHANGELOG for 0.3.0

**Summary:** Update `README.md` feature table + add static-files section; update `docs/architecture.md` with the file-body response flow; add `examples/static_files.rs` to `Cargo.toml`; write CHANGELOG 0.3.0 entry.

**Detail:** CHANGELOG entry must explicitly explain the version jump: "0.2.0 was an internal architectural checkpoint and was not published. 0.3.0 is the first public release with the full framework surface including static file serving." Link or summarize the internal 0.2.0 retrospective.

### Task 2.6 — Pre-publish gate

**Summary:** `cargo publish --dry-run`, verify `cargo test --all-features` passes, verify examples build.

**Detail:** Concrete checks before publish (user triggers actual publish):

- `cargo test` — default features.
- `cargo test --all-features` — includes openapi, ws, tls, recover, multi-thread.
- `cargo clippy --all-targets --all-features -- -D warnings` — zero warnings per CLAUDE.md.
- `cargo build --example static_files`
- `cargo build --example hello --example groups --features openapi --example tls --features tls --example sse --example ws_echo --features ws`
- `cargo doc --no-deps --all-features` — no broken intra-doc links.
- `cargo publish --dry-run` — ensures the manifest is publishable (no `path = "..."` without `version = "..."`, metadata populated).

---

## Phase 3 — Public 0.3.1 (measured performance, frozen API)

Four candidate hot-path fixes, ordered by expected impact × risk. Each must have a benchmark fixture that measurably moves before it ships. No API changes permitted — this is a patch release.

### Task 3.1 — Baseline recapture

**Summary:** Re-run benchmark harness on post-0.3.0 `main`; save `benchmarks/results/results-0.3.0.md` as the 0.3.1 reference.

**Detail:** The 0.2-era baseline (`results-2026-04-19.md`) is stale once static files land. Every 0.3.1 claim is measured against the 0.3.0 baseline, not the 0.2.0 one. Use the same fixtures and the same hardware. Add one new fixture: `empty_get_zero_middleware_zero_params` — `GET /health` returning `204 No Content` with no middleware attached. This is the fixture that Tasks 3.2 / 3.3 / 3.4 all move.

### Task 3.2 — Zero-middleware fast path (post-routing dispatch)

**Summary:** When a matched route has no middleware, skip `Next<S>` construction and call the endpoint directly.

**Detail:** `src/server.rs:384-389` currently builds a `Next` struct and calls `next.run()` unconditionally. The pre-routing side already has this fast path at `src/server.rs:322-340` — mirror it. Branch on `route.middleware.is_empty()`; in the fast path, invoke `route.endpoint.call(req, state)` directly. Unit test: with zero app middleware and a zero-middleware route, the future type returned does not transit a `Next::run`. Expected impact: one less Arc clone and one less future allocation per request on the majority of real-world handlers (most routes don't carry middleware).

### Task 3.3 — Conditional RouteParams allocation (if not already done in 2.4)

**Summary:** Guard the `RouteParams` build + extension insertion behind `matched.params.is_empty()`.

**Detail:** `src/router.rs:94-100`. If Task 2.4 already fixed this, this task is a no-op — note so in the 0.3.1 CHANGELOG as "landed in 0.3.0." Otherwise, fix here. Update any extractor that unwraps the `RequestContext` expecting params (check `src/extract/path.rs`) to handle the `None` / `is_empty` case without error — absence means "no route params," which is a valid state for parameterless routes.

### Task 3.4 — De-tax `body::empty()` on the response path

**Summary:** Avoid re-wrapping a fresh `BoxBody` on every `body::empty()` call.

**Detail:** `src/body.rs:28-37`. `empty()` currently calls `full(Bytes::new())` which calls `.boxed_unsync()`. Two options:

- **(a)** Cache an empty-body template per-thread or as a `once_cell::sync::Lazy` and clone-on-use. `BoxBody` is not `Clone`, but the underlying `http_body_util::Empty<Bytes>` is trivially constructable each time without a heap alloc for the body's data (only the `BoxBody` wrapper allocates). Verify the actual allocation cost by profiling before optimizing.
- **(b)** Introduce a response body enum (`Response<EnumBody>`) where the common variants (`Empty`, `Full(Bytes)`, `Boxed(BoxBody<...>)`) are static dispatch. This is a bigger change — likely requires handler-layer changes downstream. **Defer to 0.4 if it looks invasive.**

Start with (a). Measure. If the benchmark doesn't move, revert — allocator elision may already handle this. **Rule: if no measurable improvement on the zero-middleware fixture, do not ship the change.**

### Task 3.5 — Timer gating investigation (research task, may conclude "no change")

**Summary:** Determine whether `TokioTimer::new()` can be skipped on the hyper builder when no timeouts are configured.

**Detail:** `src/server.rs:157-158`. CLAUDE.md states the timer is required "unconditionally" because hyper panics without it when timeouts are set. But if `ServerConfig` has all timeouts as `None`, the timer may be unnecessary. Validate by:

- Check current hyper 1.x behavior: does `hyper::server::conn::http1::Builder` panic *only* when you call `keep_alive(true)` / `header_read_timeout(...)` without a timer, or also on any request?
- If the panic is conditional on timeout APIs being called, add a `has_any_timeout()` check to `ServerConfig` and gate `builder.timer(...)` on it.
- If hyper panics unconditionally without a timer, document the finding in the retrospective ("0.3.1 investigated; cannot remove — hyper constraint") and close.

This is a true research task. It may produce no code change. That is a valid outcome; write up the finding so 0.3.2 does not re-investigate.

### Task 3.6 — CHANGELOG 0.3.1 and publish gate

**Summary:** Write a perf-only CHANGELOG with measured deltas; re-run full test matrix; publish.

**Detail:** CHANGELOG entry is strictly measured — cite before/after numbers per fixture from `results-0.3.0.md` vs `results-0.3.1.md`. No hand-wavy claims. If a task shipped but didn't move the needle, state that too — honesty about negative results builds trust. Same pre-publish gate as Task 2.6 (`cargo test --all-features`, `cargo clippy -- -D warnings`, `cargo doc`, `cargo publish --dry-run`).

**Public API check:** run `cargo public-api diff` (or equivalent) against 0.3.0. Zero additions, zero removals expected — this is a patch release.

---

## Critical Files

- `/home/alexey/projects/sandbox/flowgate/Cargo.toml` — version bumps (0.3.0, then 0.3.1); add `examples/static_files.rs` entry in Phase 2.
- `/home/alexey/projects/sandbox/flowgate/CHANGELOG.md` — entries for 0.3.0 and 0.3.1. Do **not** add a 0.2.0 "Released" entry — 0.2.0 stays internal.
- `/home/alexey/projects/sandbox/flowgate/docs/architecture.md` — verify in 1.2; update in 2.5 with file-body response flow.
- `/home/alexey/projects/sandbox/flowgate/docs/releases/0.2.0-internal.md` — **new file, Phase 1.**
- `/home/alexey/projects/sandbox/flowgate/docs/superpowers/plans/YYYY-MM-DD-static-files.md` — **new sub-plan, Phase 2.**
- `/home/alexey/projects/sandbox/flowgate/README.md` — feature table update in 2.5.
- `/home/alexey/projects/sandbox/flowgate/examples/static_files.rs` — **new example, Phase 2.**
- Phase 3 hot-path files already located: `src/router.rs:94-100`, `src/server.rs:322-340` (reference), `src/server.rs:384-389`, `src/middleware/mod.rs:72-84`, `src/body.rs:28-37`, `src/server.rs:157-158`.

## Verification

**Phase 1 (internal 0.2.0):**
- `docs/releases/0.2.0-internal.md` exists and links to the perf baseline and CHANGELOG 0.2.0 section.
- `docs/architecture.md` accurately describes pre-routing middleware, WS upgrade, TLS, groups.
- `crates.io` has **no** 0.2.0 entry (external check: `cargo search flowgate` shows only 0.1.x).

**Phase 2 (public 0.3.0):**
- `cargo test --all-features` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` clean.
- `cargo run --example static_files` serves files, returns correct `Content-Type`.
- Traversal test: request path `/../etc/passwd` (or encoded equivalents `%2e%2e%2f...`) returns 404 or 400, never 200 with file contents.
- HEAD request returns same headers as GET with empty body.
- Conditional GET: second request with `If-None-Match: <etag>` returns 304.
- Range request: `curl -H "Range: bytes=0-9" http://.../file.bin` returns 206 with 10 bytes.
- SPA mode: request for `/some/deep/spa/route` falls through to configured fallback file with 200.
- `cargo publish --dry-run` succeeds.

**Phase 3 (public 0.3.1):**
- `cargo test --all-features` passes (no regressions from 0.3.0).
- `benchmarks/results/results-0.3.1.md` exists and shows measured deltas vs `results-0.3.0.md` on at least the `empty_get_zero_middleware_zero_params` fixture.
- `cargo public-api diff` (or manual inspection) shows zero API surface changes vs 0.3.0.
- For each shipped perf task: before/after numbers in CHANGELOG. For each non-shipped investigation (e.g., 3.5), a note in the release doc explaining why.

## Out of scope for this plan

- Git branching mechanics (user handles PRs and branch creation manually).
- The static-files sub-plan's per-task TDD breakdown — that plan is created in Task 2.2 and lives in its own file.
- 0.3.2+ follow-ups (multipart ranges, precompressed assets, body enum refactor from 3.4 option (b)).
