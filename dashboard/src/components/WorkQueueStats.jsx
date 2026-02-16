/**
 * WorkQueueStats component displays statistics for work queue batches
 */
const WorkQueueStats = ({ stats }) => {
  if (!stats || stats.length === 0) {
    return null;
  }

  return (
    <div style={{ marginBottom: "20px" }}>
      <h3>Work Queue Status</h3>
      <div style={{ display: "flex", gap: "15px", flexWrap: "wrap" }}>
        {stats.map((stat) => (
          <div
            key={stat.status}
            style={{
              padding: "15px",
              backgroundColor: "#fff",
              border: "1px solid #ddd",
              borderRadius: "8px",
              minWidth: "150px",
            }}
          >
            <div
              style={{
                fontSize: "12px",
                color: "#666",
                textTransform: "uppercase",
                marginBottom: "5px",
              }}
            >
              {stat.status}
            </div>
            <div style={{ fontSize: "24px", fontWeight: "bold" }}>{stat.batch_count || 0}</div>
            <div style={{ fontSize: "11px", color: "#999", marginTop: "5px" }}>
              {stat.total_samples || 0} samples
            </div>
            {stat.avg_sample_time_ms && (
              <div style={{ fontSize: "11px", color: "#999" }}>
                ~{stat.avg_sample_time_ms.toFixed(1)}ms/sample
              </div>
            )}
          </div>
        ))}
      </div>
    </div>
  );
};

export default WorkQueueStats;
