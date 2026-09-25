# Nuphus Workbench service

This crate is the edition's project, draft, version and run service. It does not
depend on the internal Agent, Tauri or a second workflow interpreter. The desktop
host supplies the upstream compiler and executor through `service::Host`.

## Current development status

Implemented: project registration, revision-checked IR editing, independent layout
revisions, immutable published versions, run identity and idempotency, durable
cursor events, scoped client tokens, local HTTP/MCP and a stdio bridge.

The full Workbench UI, internal-generation integration, public debug/automation
adapters and release/synchronization workflows are still under development.
Do not treat passing service tests as desktop or macOS acceptance.

## Transports

The Workbench application/tray hosts `http://127.0.0.1:47731` (override port using
`NUPHUS_WORKBENCH_PORT`). It does not start an independent privileged daemon.
Bind failures are exposed to the UI rather than silently choosing another service.
All endpoints require `Authorization: Bearer <client token>` created by the local
UI. No model key is an API access token. Tokens are stored hashed, displayed once,
project scoped and revocable. Revocation is checked on every request, including
requests in existing MCP sessions and continued event streams.

* `GET /api/v1/discover`: operation names and JSON input schemas.
* `POST /api/v1/{operation}`: operation arguments as JSON. Responses contain
  `result`, or `error: {code, message, details?}`.
* `GET /api/v1/events?project_id=...&after=...&run_id=...`: SSE; `run_id` optional.
  Reconnect using the last processed cursor in `after`. `run.events` provides the
  same durable data for polling/MCP; transport session IDs are not run IDs.
* `/mcp`: MCP Streamable HTTP via the official Rust SDK. Tool names replace dots
  with underscores, e.g. `canvas_update`, `workflow_run`, `run_events`.
* `nuphus-workbench-mcp`: stdio bridge using `NUPHUS_WORKBENCH_URL` (loopback origin)
  and `NUPHUS_WORKBENCH_TOKEN`. It proxies the same service; it does not open a
  second database or execute work itself. Stdout is reserved for MCP.

Remote clients can use a separately authenticated tunnel to this local endpoint;
arbitrary network binding and access from web-page origins are not enabled.

## Workflow lifecycle

1. `project.list` → select a permitted project (or explicitly register one).
2. `canvas.create` → obtain workflow ID and content revision. External clients
   default to `external` authoring; the local UI defaults to `internal`.
3. `canvas.update` → apply a batch with the exact last-read revision. Nested
   steps use upstream IR (`seq`, `if.then/else`, `loop.do`, `wait/auto`); visual
   edges are derived from this IR, not separately executable edges.
4. `workflow.validate` → native compiler diagnostics. Incomplete drafts may be
   saved as drafts, but cannot be published/executed until validation passes.
5. `workflow.save` → publish an immutable version at that revision.
6. `workflow.run` → provide version ID, runtime inputs and a caller request ID.
   It immediately returns a real durable run ID. The published root and child
   definitions are frozen for that run; later edits affect only future runs.
7. `run.get` / `run.events` / `run.steps` → status, replayable progress and evidence.
   `run.respond` binds a response to a particular pending wait request; it cannot
   approve a later loop iteration. Use `run.cancel` to request cancellation.

A connection ending does not cancel execution. Retry a start with the **same**
`request_id` and identical arguments after an uncertain response; changed arguments
with that ID are rejected. A new ID intentionally creates a new execution and may
repeat side effects. Host restart marks unfinished runs interrupted, never replays
them automatically. Failed runs remain queryable.

Content conflicts return `revision_conflict` and never overwrite another editor.
Layout has its own revision. Deleting a draft preserves versions and run evidence.
Client capabilities are `read`, `edit`, `run`, `respond`, `automation`, `projects`;
grant only those needed, without prompting users for every ordinary workflow node.

## Checks

```powershell
cargo test -p nuphus-workbench --features gateway
cargo clippy -p nuphus-workbench --features gateway --all-targets -- -D warnings
```
