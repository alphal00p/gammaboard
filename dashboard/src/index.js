import React from "react";
import ReactDOM from "react-dom/client";
import { ThemeProvider, createTheme } from "@mui/material/styles";
import CssBaseline from "@mui/material/CssBaseline";
import "./index.css";
import App from "./App";

const theme = createTheme();
const strictModeEnabled = process.env.REACT_APP_STRICT_MODE !== "false";
const RootWrapper = strictModeEnabled ? React.StrictMode : React.Fragment;

const root = ReactDOM.createRoot(document.getElementById("root"));
root.render(
  <RootWrapper>
    <ThemeProvider theme={theme}>
      <CssBaseline />
      <App />
    </ThemeProvider>
  </RootWrapper>,
);
