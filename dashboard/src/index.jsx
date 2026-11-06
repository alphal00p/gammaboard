import React from "react";
import ReactDOM from "react-dom/client";
import "./index.css";
import App from "./App";

const strictModeEnabled = import.meta.env.VITE_STRICT_MODE !== "false";
const RootWrapper = strictModeEnabled ? React.StrictMode : React.Fragment;

const root = ReactDOM.createRoot(document.getElementById("root"));
root.render(
  <RootWrapper>
    <App />
  </RootWrapper>,
);
