import { createContext, Suspense, useContext, useEffect, useState } from "react";
import { Box, Tab, Tabs } from "@mui/material";
import LoadingStatus from "./LoadingStatus";

const WorkspaceActivity = createContext(true);
export const useWorkspaceActive = () => useContext(WorkspaceActivity);

// Retain controllers, drafts and uploaded comparisons, but let their polling
// and chart renderers suspend while another workspace is visible.
export function WorkspacePanel({ id, value, active, children }) {
  const parentActive = useWorkspaceActive();
  const [visited, setVisited] = useState(active);
  useEffect(() => { if (active) setVisited(true); }, [active]);
  return <Box role="tabpanel" id={`${id}-panel-${value}`} aria-labelledby={`${id}-tab-${value}`} hidden={!active} sx={{ minWidth: 0 }}>
    {(active || visited) && <WorkspaceActivity.Provider value={parentActive && active}>
      <Suspense fallback={<LoadingStatus loading label="Loading view…" />}>{children}</Suspense>
    </WorkspaceActivity.Provider>}
  </Box>;
}

export default function WorkspaceTabs({ id, label, value, onChange, tabs, level = "section", panelId }) {
  const primary = level === "primary";
  return <Tabs value={value} onChange={(_, next) => onChange(next)} aria-label={label}
    variant="scrollable" scrollButtons="auto" allowScrollButtonsMobile
    sx={{ mb: 2, minHeight: primary ? 56 : 48,
      borderBottom: level === "section" ? 1 : 0, borderColor: "divider",
      "& .MuiTabs-flexContainer": { width: "max-content", minWidth: "100%", justifyContent: "center", gap: primary ? 1 : 0 },
      "& .MuiTabs-indicator": { display: level === "section" ? "block" : "none" },
      "& .MuiTab-root": { minWidth: 0, px: { xs: 1.5, sm: 2.5 }, textTransform: "none",
        fontSize: primary ? "1rem" : "0.875rem", borderRadius: level === "section" ? 0 : 1,
        "&.Mui-selected": primary ? { bgcolor: "primary.main", color: "primary.contrastText" }
          : level === "detail" ? { bgcolor: "action.selected", color: "text.primary" } : {},
      },
    }}>
    {tabs.map(([key, text]) => <Tab key={key} value={key} label={text}
      id={`${id}-tab-${key}`} aria-controls={panelId || `${id}-panel-${key}`} />)}
  </Tabs>;
}
