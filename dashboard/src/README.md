# Dashboard Architecture

## Structure

```
src/
├── components/       # UI components
├── context/          # Centralized run history provider
├── hooks/            # Custom React hooks for data fetching
├── services/         # API client
├── utils/            # Helper functions
├── App.js            # Main app
└── index.js          # Entry point
```

## Data Flow

```
Backend API → context/RunHistoryContext → App.js → components
```

## Components

- **ConnectionStatus** - Shows backend connection status
- **RunSelector** - Dropdown to select runs
- **RunInfo** - Grid of run metrics
- **WorkQueueStats** - Batch status cards
- **SampleChart** - Line chart with Recharts

## State Management

- **RunHistoryProvider** - Central source of truth for run details, aggregated history, worker logs, and live updates.
- **useRunState / useRunConnection / useRunHeartbeat** - Split hooks to reduce unnecessary rerenders.
- **useRunHistory** - Combined convenience hook.

## Hooks

- **useRuns(refreshInterval)** - Fetches runs list, manages selection (default: 2s refresh)

## Configuration

**API URL:** set `REACT_APP_API_BASE_URL`.

**Polling + retention:** Edit `DASHBOARD_HISTORY_CONFIG` in `App.js`.
- `historyLimit`
- `historyBufferMax`
- `workerLogsLimit`
- `workQueueStatsLimit`
- `pollIntervalMs`
- `sseConnectedPollThrottleFactor`
- `streamIntervalMs`

## Tech Stack

- React 19.2.4
- Material UI 5.x (default theme)
- Recharts 3.7.0
