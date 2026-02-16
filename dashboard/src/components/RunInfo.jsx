/**
 * RunInfo component displays detailed information about the selected run
 */
const RunInfo = ({ run }) => {
  if (!run) {
    return null;
  }

  return (
    <div
      style={{
        marginBottom: "20px",
        padding: "15px",
        backgroundColor: "#f5f5f5",
        borderRadius: "8px",
        display: "grid",
        gridTemplateColumns: "repeat(auto-fit, minmax(200px, 1fr))",
        gap: "15px",
      }}
    >
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Status</div>
        <div style={{ fontSize: "18px", fontWeight: "bold" }}>{run.run_status || "unknown"}</div>
      </div>
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Total Batches</div>
        <div style={{ fontSize: "18px", fontWeight: "bold" }}>{run.total_batches || 0}</div>
      </div>
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Total Samples</div>
        <div style={{ fontSize: "18px", fontWeight: "bold" }}>{run.total_samples || 0}</div>
      </div>
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Completed Batches</div>
        <div style={{ fontSize: "18px", fontWeight: "bold", color: "#4caf50" }}>
          {run.completed_batches || 0}
        </div>
      </div>
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Pending Batches</div>
        <div style={{ fontSize: "18px", fontWeight: "bold", color: "#ff9800" }}>
          {run.pending_batches || 0}
        </div>
      </div>
      <div>
        <div style={{ fontSize: "12px", color: "#666" }}>Completion Rate</div>
        <div style={{ fontSize: "18px", fontWeight: "bold" }}>
          {run.completion_rate ? `${(run.completion_rate * 100).toFixed(1)}%` : "0%"}
        </div>
      </div>
    </div>
  );
};

export default RunInfo;
