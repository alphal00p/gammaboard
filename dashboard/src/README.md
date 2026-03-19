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

- `TaskOutputPanel` renders the selected task using task-owned panel descriptors, task-local persisted history, and a small frontend metadata header for task identity/progress.
- `PerformanceWorkspace` renders run-level sampler throughput panels plus a selected evaluator worker's timing/idle panels through the same panel vocabulary.
- Engine config panels such as evaluator and sampler aggregator are fetched as backend-generated generic panels and should avoid reconstructing task-specific runtime state in the frontend.
- `PanelCollection` is the shared frontend renderer. One component exists per panel kind.

## Hooks

- `useRuns()` polls the run list.
- `useRunTasks(runId)` polls task state for the selected run.
- `useTaskOutput({ runId, taskId })` polls the selected task output plus incremental persisted history.
- `useRunPerformancePanels({ runId, evaluatorNodeId })` polls run-level sampler performance plus the selected evaluator worker's performance panels.
- `useRunConfigPanels({ runId })` polls backend-generated evaluator and sampler config panels for the selected run.
- `useWorkerLogs()` fetches log history for the Logs tab.

## Configuration

Set `REACT_APP_API_BASE_URL`.

## Tech Stack

- React 19.2.4
- Material UI 5.x
- Recharts 3.7.0
