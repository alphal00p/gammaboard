import ConnectionStatus from "./components/ConnectionStatus";
import RunSelector from "./components/RunSelector";
import RunInfo from "./components/RunInfo";
import WorkQueueStats from "./components/WorkQueueStats";
import SampleChart from "./components/SampleChart";
import { useRuns } from "./hooks/useRuns";
import { useRunData } from "./hooks/useRunData";

function App() {
  // Fetch runs and manage selection
  const { runs, selectedRun, setSelectedRun, isConnected } = useRuns();

  // Fetch data for selected run
  const { samples, stats, lastUpdate } = useRunData(selectedRun);

  // Find current run details
  const currentRun = runs.find((r) => r.run_id === selectedRun);

  return (
    <div style={{ padding: "20px", fontFamily: "Arial, sans-serif", maxWidth: "1400px", margin: "0 auto" }}>
      <h1 style={{ marginBottom: "10px" }}>Gammaboard</h1>

      <ConnectionStatus isConnected={isConnected} lastUpdate={lastUpdate} />

      <RunSelector runs={runs} selectedRun={selectedRun} onRunChange={setSelectedRun} />

      <RunInfo run={currentRun} />

      <WorkQueueStats stats={stats} />

      <SampleChart samples={samples} isConnected={isConnected} currentRun={currentRun} />
    </div>
  );
}

export default App;
