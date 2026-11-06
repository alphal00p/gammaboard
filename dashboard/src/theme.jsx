import { CssBaseline, IconButton, Tooltip } from "@mui/material";
import DarkModeOutlined from "@mui/icons-material/DarkModeOutlined";
import LightModeOutlined from "@mui/icons-material/LightModeOutlined";
import { createTheme, ThemeProvider, useColorScheme } from "@mui/material/styles";

const theme = createTheme({
  colorSchemes: { light: true, dark: true },
  shape: { borderRadius: 8 },
  typography: {
    h5: { fontSize: "1.375rem", fontWeight: 600 },
    h6: { fontSize: "1.125rem", fontWeight: 600 },
    subtitle1: { fontSize: "0.9375rem", fontWeight: 600 },
    subtitle2: { fontSize: "0.875rem", fontWeight: 600 },
    button: { textTransform: "none", fontWeight: 500 },
  },
  components: {
    MuiButton: {
      defaultProps: { disableElevation: true },
      styleOverrides: { root: { minHeight: 40, whiteSpace: "nowrap" }, sizeSmall: { minHeight: 32 } },
    },
    MuiIconButton: { defaultProps: { size: "small" } },
    MuiTab: { styleOverrides: { root: { textTransform: "none", minHeight: 44, padding: "10px 16px" } } },
    MuiTabs: { styleOverrides: { root: { minHeight: 44 } } },
    MuiTextField: { defaultProps: { size: "small" } },
    MuiFormControl: { defaultProps: { size: "small" } },
    MuiChip: { defaultProps: { size: "small", variant: "outlined" }, styleOverrides: { root: { fontWeight: 500 } } },
    MuiCard: { defaultProps: { variant: "outlined" } },
    MuiCardContent: { styleOverrides: { root: { padding: 20, "&:last-child": { paddingBottom: 20 } } } },
    MuiTableCell: { styleOverrides: {
      root: { fontVariantNumeric: "tabular-nums" },
      head: { fontWeight: 600, color: "inherit", whiteSpace: "nowrap" },
    } },
    MuiFormControlLabel: { styleOverrides: { label: { fontSize: "0.875rem" } } },
  },
});

export function DashboardTheme({ children }) {
  return (
    <ThemeProvider theme={theme} defaultMode="light" modeStorageKey="gammaboard.theme" disableTransitionOnChange noSsr>
      <CssBaseline enableColorScheme />
      {children}
    </ThemeProvider>
  );
}

export function ThemeToggle() {
  const { mode, systemMode, setMode } = useColorScheme();
  const dark = (mode === "system" ? systemMode : mode) === "dark";
  const label = dark ? "Switch to light theme" : "Switch to dark theme";
  return (
    <Tooltip title={label}>
      <IconButton aria-label={label} color="inherit" onClick={() => setMode(dark ? "light" : "dark")}>
        {dark ? <LightModeOutlined /> : <DarkModeOutlined />}
      </IconButton>
    </Tooltip>
  );
}
