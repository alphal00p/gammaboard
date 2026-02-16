# Dashboard Architecture

This dashboard follows React best practices for component organization and separation of concerns.

## Project Structure

```
src/
├── components/          # Reusable UI components
│   ├── ConnectionStatus.jsx    # Shows connection state and last update time
│   ├── RunSelector.jsx         # Dropdown to select which run to view
│   ├── RunInfo.jsx             # Display run statistics and metadata
│   ├── WorkQueueStats.jsx      # Display work queue batch statistics
│   └── SampleChart.jsx         # Line chart for sample visualization
├── hooks/               # Custom React hooks for data fetching
│   ├── useRuns.js              # Fetches and manages list of runs
│   └── useRunData.js           # Fetches samples and stats for a specific run
├── services/            # API communication layer
│   └── api.js                  # All backend API calls
├── utils/               # Helper functions
│   └── sampleParser.js         # Parses raw samples into chart format
├── App.js              # Main app orchestrator (thin)
└── index.js            # App entry point
```

## Key Principles

### 1. Component-Based Architecture
Each UI element is a separate, focused component:
- **ConnectionStatus**: Connection indicator and last update time
- **RunSelector**: Run selection dropdown
- **RunInfo**: Run metadata display grid
- **WorkQueueStats**: Work queue status cards
- **SampleChart**: Recharts visualization with tooltip

### 2. Custom Hooks for Data Management
Data fetching logic is extracted into custom hooks:
- **useRuns**: Manages runs list, auto-selection, and connection state
- **useRunData**: Manages samples and stats for selected run

### 3. Service Layer
All API calls are centralized in `services/api.js`:
- `fetchRuns()` - Get all runs
- `fetchSamples(runId, limit)` - Get samples for a run
- `fetchStats(runId)` - Get statistics for a run

### 4. Utility Functions
Helper functions are isolated in the `utils/` directory:
- `parseSamples()` - Transforms raw API data into chart-ready format

## Benefits

### For AI-Assisted Development
- **Targeted changes**: "Update the SampleChart component to add a new feature"
- **Clear boundaries**: Each file has a single responsibility
- **Easy to understand**: Small, focused files are easier for AI to comprehend

### For Manual Development
- **Easy navigation**: Find what you need quickly
- **Clear dependencies**: Import statements show component relationships
- **Testable**: Each piece can be tested in isolation
- **Maintainable**: Changes to one component don't affect others

## Making Changes

### Adding a New Component
1. Create a new file in `src/components/ComponentName.jsx`
2. Export the component as default
3. Import and use it in `App.js`

### Adding a New API Endpoint
1. Add the function to `src/services/api.js`
2. Use it in a custom hook or component

### Adding a New Hook
1. Create a new file in `src/hooks/useSomething.js`
2. Return the state and functions needed by components
3. Use it in `App.js` or other components

### Modifying a Component
1. Open the specific component file
2. Make your changes
3. The component is self-contained, so changes won't affect others

## Example Workflows

### Working with AI to Add a Feature
```
You: "Add a dark mode toggle to the ConnectionStatus component"
AI: *modifies only src/components/ConnectionStatus.jsx*
```

### Working with AI to Add New Data
```
You: "Fetch and display worker status from /api/workers"
AI: 
1. Adds fetchWorkers() to src/services/api.js
2. Creates useWorkers hook in src/hooks/useWorkers.js
3. Creates WorkerStatus component in src/components/WorkerStatus.jsx
4. Updates App.js to use the new component
```

### Manual Changes
- Want to change the chart colors? Edit `src/components/SampleChart.jsx`
- Need to adjust refresh rate? Edit `src/hooks/useRunData.js`
- Want to change API base URL? Edit `src/services/api.js`

## Component Props Reference

### ConnectionStatus
- `isConnected` (boolean) - Connection state
- `lastUpdate` (string) - Last update timestamp

### RunSelector
- `runs` (array) - List of runs
- `selectedRun` (number) - Currently selected run ID
- `onRunChange` (function) - Callback when selection changes

### RunInfo
- `run` (object) - Run details object

### WorkQueueStats
- `stats` (array) - Array of work queue statistics

### SampleChart
- `samples` (array) - Array of parsed sample data
- `isConnected` (boolean) - Connection state
- `currentRun` (object) - Current run details

## Configuration

### API Base URL
Edit `src/services/api.js`:
```javascript
const API_BASE_URL = "http://localhost:4000/api";
```

### Refresh Intervals
Edit the hook calls in `src/App.js`:
```javascript
const { runs, ... } = useRuns(2000);  // 2 second refresh
const { samples, ... } = useRunData(selectedRun, 1000);  // 1 second refresh
```
