# Frontend

The dashboard is a React/Vite app that renders backend-owned run, task, worker,
and log read models. The frontend should stay generic: server responses define
panel content and layout semantics, while React handles selection, polling, and
presentation.

## Quick Links

- Dashboard package: `dashboard/`
- App entry point: `dashboard/src/App.jsx`
- API client: `dashboard/src/services/api.js`
- Local dev proxy: `dashboard/vite.config.js`

## Structure

```text
dashboard/src/
  components/       UI components and workspace views
    runs/            Run actions, dialogs, and run-scoped workspace
  hooks/            Polling/data hooks
  services/         API client
  utils/            Formatting and view-model helpers
  App.jsx           Navigation and top-level selection shell
  index.jsx         Entry point
```

## Data Flow

```text
Backend panel poll endpoints -> usePanelSource -> PanelCollection -> renderers
```

- `TaskOutputPanel` renders the selected task from one server-owned poll
  response containing panel specs plus `replace` and `append` updates.
- `PerformanceWorkspace` has Overview, Diagnostics, and Graphs views.
  Overview and Diagnostics use the latest 60 seconds. Graphs has full recorded
  history with one linked time selection for activity and throughput; there is
  no window dropdown.
  Overview shows accepted progress/rate, live-worker reporting coverage, busy
  fractions, allocated core-hours, GammaBoard process RSS with coverage, and age.
  Diagnostics holds worker intervals, queue/batch state, operation timings,
  and queue tuning.
  Graphs shows the four compute/I/O activity traces and accepted throughput.
- The effective engine config uses one stage-aware panel response for both the
  evaluator and sampler and normally only emits `replace` updates.
- `usePanelSource` owns cursor tracking and patch application.
- `PanelCollection` renders panel state and applies simple layout hints.
- `RunInfo` uses backend-generated run summary panels instead of parsing run
  config in the browser.

## Hooks

- `useRuns()` polls the run list.
- `useRunTasks(runId)` polls task state for the selected run.
- `useTaskOutput({ runId, taskId })` polls selected task panels with the
  server-owned opaque cursor.
- `useRunPerformancePanels({ runId, evaluatorNodeName, windowSeconds })` polls live
  performance panels. `PerformanceGraphs` fetches full history plus finer detail
  for the selected time range, only while the Graphs view is open.
- `useRunPanels({ runId })` polls the backend-generated run summary and
  effective engine config panels.
- `useWorkerLogs()` fetches log history for the Logs tab.

## API Routing

The API base URL is the Vite application base followed by `/api`, as defined in
`dashboard/src/services/api.js`. For local development, Vite proxies `/api` to
`http://127.0.0.1:4000`.

Server-side node startup requests always go through the generic launch-request
queue. The frontend does not branch on local vs external spawning.

## Measurement semantics

The performance endpoint accepts `window_seconds` (default 60, bounded to
15–300); the dashboard uses 60 seconds. Current usage requires a live matching
identity and task. Publications are asynchronous: the
requested bounds are shared, while each worker's actual observed span is shown
in Diagnostics. Rate and busy fractions require two current-epoch reports;
warm-up, resets, missing reports, and reports older than ten seconds yield
unavailable values. A failed refresh or ten seconds without a successful
response hides the previous measurements. Measured zero remains zero.

Accepted rate uses the sampler's accepted-sample counter difference over its
observed interval. The Activity table has two rows (Evaluators and Sampler) and
two measurements per row (Compute busy and I/O active). Both use cumulative
occupied seconds and elapsed seconds from one monotonic clock per worker.
Evaluator rates weight measured worker-time and require all live evaluators to
report. Missing/legacy counters are unavailable, never inferred from slot counts.

Compute covers materialization/evaluation or generation/training/merge calls,
including failed attempts. I/O covers work-related database operations, including
connection/query waits. Its timer begins inside the operation and ends on actual
completion, error, or cancellation. Queued tasks and finished results awaiting
collection are excluded. Concurrent I/O operations count once; compute and I/O
can overlap. Heartbeats, control polling, and telemetry publication are excluded.
These are wall-time activity fractions, not CPU, GPU, or database saturation.

`GET /runs/:id/performance/graphs` returns full recorded history by default;
optional `start_ms` and `end_ms` select a viewport with no duration cap. Each
response has at most 600 display intervals per trace and reads compact counter
records in pages rather than retaining the entire raw history in server memory.
One report on either side of the selection preserves intervals across its edges.

The two graphs share draggable time sliders. Zooming requests finer detail;
Full history restores the complete range. Past selections stay fixed in absolute
time, while a selection at the right edge follows new reports at constant width.
Graph refresh is five seconds; the coarse navigation history refreshes every
30 seconds while zoomed. Query cost grows with the number of selected raw reports,
although response size and plotting cost stay bounded. Historical data remains
visible with a warning if refreshing fails.

Graphs keep four stable traces with 0–100% axes and average measured worker-time.
No min/max overlay or smoothing is applied. Identity changes break lines and
missing measurements remain gaps. Completed runs retain their history. The
observed report spacing and display-bin width are shown separately: smaller bins
cannot recover detail below the original reporting interval. The overview
requires full live-worker coverage; historical fleet means cover reporting workers.

Both worker roles default to `performance_snapshot_interval_ms = 2000`. A shorter
configured interval is a target, not a guaranteed cadence: snapshots run within
the worker loop, so synchronous evaluation or sampler updates delay publication.
Busy counters still account for occupied wall time across those longer intervals.
For subsecond minibatch pauses, first inspect the observed spacing. Faster
publication can help only when the worker loop returns often enough; smoothing
would suppress the same short pauses. The graph does not manufacture finer data.

Allocated core-hours measure allocation, not consumed CPU time. RSS sums fresh
GammaBoard worker reports with explicit coverage; children, GPUs, and services
are excluded. Diagnostic operation means divide duration totals by the relevant
sample/batch/operation count. Overlapping durations are not added into a pipeline
breakdown. There is no separate tick-busy or retained-slot-occupancy percentage.

See [concurrency.md](concurrency.md) for worker overlap, training barriers, and
how to interpret bottlenecks.

## Logs Tab

The logs tab reads `GET /api/logs` with a `run_id` filter.

- Filters: `node_name`, `level`, `q`
- Cursor pagination: `before_id`
- Response shape: `{ items, next_before_id, has_more_older }`
- UI model: read-only table with `Refresh` and `Load older`

## Tech Stack

- React 19.2.4
- Vite
- Material UI 7.x
- ECharts
