import { LineChart, Line, XAxis, YAxis, Tooltip, CartesianGrid } from "recharts";

/**
 * SampleChart component displays a line chart of sample values
 */
const SampleChart = ({ samples, isConnected, currentRun }) => {
  if (samples.length === 0) {
    return (
      <div style={{ marginTop: "30px" }}>
        <h3>Sample Values</h3>
        <div
          style={{
            padding: "40px",
            textAlign: "center",
            color: "#999",
            border: "2px dashed #ddd",
            borderRadius: "8px",
          }}
        >
          {isConnected && currentRun
            ? "No completed samples yet. Waiting for batches to be evaluated..."
            : "Select a run to view data"}
        </div>
      </div>
    );
  }

  return (
    <div style={{ marginTop: "30px" }}>
      <h3>Sample Values</h3>
      <LineChart width={1200} height={500} data={samples}>
        <CartesianGrid strokeDasharray="3 3" />
        <Line
          type="monotone"
          dataKey="value"
          stroke="#8884d8"
          dot={false}
          isAnimationActive={false}
          strokeWidth={2}
        />
        <XAxis
          dataKey="x"
          label={{ value: "x", position: "insideBottom", offset: -5 }}
          type="number"
          domain={["dataMin", "dataMax"]}
        />
        <YAxis label={{ value: "f(x)", angle: -90, position: "insideLeft" }} />
        <Tooltip
          content={({ active, payload }) => {
            if (active && payload && payload.length) {
              const data = payload[0].payload;
              return (
                <div
                  style={{
                    backgroundColor: "white",
                    padding: "10px",
                    border: "1px solid #ccc",
                    borderRadius: "4px",
                  }}
                >
                  <p style={{ margin: "0 0 5px 0" }}>
                    <strong>x:</strong> {data.x.toFixed(4)}
                  </p>
                  <p style={{ margin: "0 0 5px 0" }}>
                    <strong>value:</strong> {data.value.toFixed(6)}
                  </p>
                  <p style={{ margin: 0 }}>
                    <strong>weight:</strong> {data.weight.toFixed(4)}
                  </p>
                </div>
              );
            }
            return null;
          }}
        />
      </LineChart>
    </div>
  );
};

export default SampleChart;
