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
├── hooks/           # useRuns, useRunData
├── services/        # API client (fetchRuns, fetchSamples, fetchStats)
├── utils/           # parseSamples helper
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
useRunData(runId, 1000) // Poll data every 1 second
```

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
