/**
 * ConnectionStatus component displays the connection status and last update time
 */
const ConnectionStatus = ({ isConnected, lastUpdate }) => {
  return (
    <div style={{ marginBottom: "20px", display: "flex", alignItems: "center", gap: "20px" }}>
      <div style={{ display: "flex", alignItems: "center" }}>
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
      </div>
      {lastUpdate && <span style={{ color: "#666" }}>Last update: {lastUpdate}</span>}
    </div>
  );
};

export default ConnectionStatus;
