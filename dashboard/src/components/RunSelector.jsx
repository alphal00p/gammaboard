/**
 * RunSelector component allows users to select which run to view
 */
const RunSelector = ({ runs, selectedRun, onRunChange }) => {
  if (runs.length === 0) {
    return null;
  }

  return (
    <div style={{ marginBottom: "20px" }}>
      <label style={{ fontWeight: "bold", marginRight: "10px" }}>Select Run:</label>
      <select
        value={selectedRun || ""}
        onChange={(e) => onRunChange(Number(e.target.value))}
        style={{
          padding: "8px",
          fontSize: "14px",
          borderRadius: "4px",
          border: "1px solid #ddd",
        }}
      >
        {runs.map((run) => (
          <option key={run.run_id} value={run.run_id}>
            Run #{run.run_id} - {run.run_status} ({run.total_batches || 0} batches)
          </option>
        ))}
      </select>
    </div>
  );
};

export default RunSelector;
