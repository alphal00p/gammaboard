import { useState, useEffect } from "react";
import { LineChart, Line, XAxis, YAxis, Tooltip, CartesianGrid } from "recharts";

function App() {
  const [results, setResults] = useState([]);
  const [isConnected, setIsConnected] = useState(false);
  const [lastUpdate, setLastUpdate] = useState(null);

  useEffect(() => {
    // Function to fetch results
    const fetchResults = async () => {
      try {
        const res = await fetch("http://localhost:4000/api/results");
        const data = await res.json();

        if (Array.isArray(data)) {
          setResults(data);
          setIsConnected(true);
          setLastUpdate(new Date().toLocaleTimeString());
        } else {
          console.error("Expected an array from backend", data);
          setResults([]);
        }
      } catch (err) {
        console.error("Failed to fetch results:", err);
        setIsConnected(false);
      }
    };

    // Fetch immediately
    fetchResults();

    // Set up polling interval (every 1 second)
    const interval = setInterval(fetchResults, 1000);

    // Cleanup interval on unmount
    return () => clearInterval(interval);
  }, []);

  return (
    <div style={{ padding: "20px", fontFamily: "Arial, sans-serif" }}>
      <h1>Gammaboard Dashboard</h1>

      <div style={{ marginBottom: "20px" }}>
        <span
          style={{
            display: "inline-block",
            width: "10px",
            height: "10px",
            borderRadius: "50%",
            backgroundColor: isConnected ? "#4caf50" : "#f44336",
            marginRight: "8px",
          }}
        ></span>
        <span style={{ fontWeight: "bold" }}>{isConnected ? "Connected" : "Disconnected"}</span>
        {lastUpdate && <span style={{ marginLeft: "20px", color: "#666" }}>Last update: {lastUpdate}</span>}
        <span style={{ marginLeft: "20px", color: "#666" }}>Total samples: {results.length}</span>
      </div>

      {results.length > 0 ? (
        <LineChart width={900} height={500} data={results}>
          <CartesianGrid strokeDasharray="3 3" />
          <Line type="monotone" dataKey="value" stroke="#8884d8" dot={false} isAnimationActive={false} />
          <XAxis dataKey="step" label={{ value: "Step", position: "insideBottom", offset: -5 }} />
          <YAxis label={{ value: "Value", angle: -90, position: "insideLeft" }} />
          <Tooltip />
        </LineChart>
      ) : (
        <div
          style={{
            padding: "40px",
            textAlign: "center",
            color: "#999",
            border: "2px dashed #ddd",
            borderRadius: "8px",
          }}
        >
          {isConnected ? "No data yet. Waiting for samples..." : "Connecting to backend..."}
        </div>
      )}
    </div>
  );
}

export default App;
