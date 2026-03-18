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
Backend panel endpoints → hooks → PanelCollection → panel renderers
```

## Main Concepts

- `TaskOutputPanel` renders the selected task using task-owned panel descriptors and task-local persisted history.
- `PerformanceWorkspace` renders evaluator and sampler performance through the same panel vocabulary.
- `PanelCollection` is the shared frontend renderer. One component exists per panel kind.

## Hooks

- `useRuns()` polls the run list.
- `useRunTasks(runId)` polls task state for the selected run.
- `useTaskOutput({ runId, taskId })` polls the selected task output plus incremental persisted history.
- `useRunPerformancePanels({ runId })` polls evaluator and sampler performance panel payloads.
- `useWorkerLogs()` fetches log history for the Logs tab.

## Configuration

Set `REACT_APP_API_BASE_URL`.

## Tech Stack

- React 19.2.4
- Material UI 5.x
- Recharts 3.7.0
