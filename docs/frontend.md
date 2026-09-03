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
  hooks/            Polling/data hooks
  services/         API client
  utils/            Formatting and view-model helpers
  App.jsx           Main app shell
  index.jsx         Entry point
```

## Data Flow

```text
Backend panel poll endpoints -> usePanelSource -> PanelCollection -> renderers
```

- `TaskOutputPanel` renders the selected task from one server-owned poll
  response containing panel specs plus `replace` and `append` updates.
- `PerformanceWorkspace` renders run-level sampler throughput and selected
  evaluator worker timing panels through the same panel transport.
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
- `useRunPerformancePanels({ runId, evaluatorNodeId })` polls performance
  panels.
- `useRunConfigPanels({ runId })` polls the backend-generated effective engine
  config panels.
- `useWorkerLogs()` fetches log history for the Logs tab.

## API Routing

The API base URL is fixed to relative `/api` in `dashboard/src/services/api.js`.
For local development, Vite proxies `/api` to `http://127.0.0.1:4000`.

Server-side node startup requests always go through the generic launch-request
queue. The frontend does not branch on local vs external spawning.

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
