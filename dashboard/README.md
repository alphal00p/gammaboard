# Gammaboard Dashboard

React dashboard for monitoring Monte Carlo simulation runs.

## Quick Start

```bash
npm install
npm start
```

Opens at http://localhost:3000

## Structure

```
src/
├── components/       # UI components (ConnectionStatus, RunSelector, etc.)
├── hooks/           # shared polling hooks and read-only log browser state
├── services/        # API client (runs, workers, history, logs)
├── utils/           # formatting and viewmodel helpers
└── App.js           # Main app
```

## Configuration

**API URL:** `src/services/api.js`
```javascript
const API_BASE_URL = process.env.REACT_APP_API_BASE_URL;
```

**Refresh Intervals:** `src/App.js`
```javascript
useRuns(2000)           // Poll runs every 2 seconds
useWorkersData(3000)    // Poll workers once app-wide every 3 seconds
```

**Logs Tab:** `GET /api/runs/:id/logs`
- Server-side filters: `worker_id`, `level`, `q`
- Cursor pagination: `before_id`
- Response shape: `{ items, next_before_id, has_more_older }`
- UI model: read-only table with `Refresh` and `Load older`

## Tech Stack

- React 19.2.4
- Material UI 5.x (default theme)
- Recharts 3.7.0

## Scripts

```bash
npm start       # Development server
npm build       # Production build
npm test        # Run tests
```
