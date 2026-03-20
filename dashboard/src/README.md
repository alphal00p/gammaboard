# Dashboard Architecture

## Structure

```
src/
├── components/       # Workspace and engine/task UI
├── hooks/            # Polling/data hooks
├── services/         # API client
├── utils/            # Small formatting/config helpers
├── App.js            # Main app shell
└── index.js          # Entry point
```

## Data Flow

```
Backend panel poll endpoints → `usePanelSource` → PanelCollection → panel renderers
```

## Main Concepts

- `TaskOutputPanel` renders the selected task using one server-owned poll response that includes panel specs plus `replace`/`append` updates.
- `PerformanceWorkspace` renders run-level sampler throughput panels plus a selected evaluator worker's timing/idle panels through the same shared panel transport.
- Engine config panels such as evaluator and sampler aggregator use the same generic panel response, but only emit `replace` updates.
- `usePanelSource` owns cursor tracking and patch application. `PanelCollection` only renders the resulting panel state.

## Hooks

- `useRuns()` polls the run list.
- `useRunTasks(runId)` polls task state for the selected run.
- `useTaskOutput({ runId, taskId })` polls the selected task panel source with the server-owned opaque `cursor`.
- `useRunPerformancePanels({ runId, evaluatorNodeId })` polls run-level sampler performance plus the selected evaluator worker's performance panels.
- `useRunConfigPanels({ runId })` polls backend-generated evaluator and sampler config panels for the selected run.
- `RunInfo` now uses backend-generated run summary panels instead of frontend config parsing.
- `useWorkerLogs()` fetches log history for the Logs tab.

## Configuration

Set `REACT_APP_API_BASE_URL`.

## Tech Stack

- React 19.2.4
- Material UI 5.x
- Recharts 3.7.0
