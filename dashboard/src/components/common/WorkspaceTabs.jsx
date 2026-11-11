import { Suspense, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { Box, Tab, Tabs } from "@mui/material";
import LoadingStatus from "./LoadingStatus";
import { TabLoadingContext, WorkspaceActivity } from "./workspaceContext";

export const useWorkspaceActive = () => useContext(WorkspaceActivity);

// Descendant loading states share the nearest tab row instead of inserting
// temporary rows above their content. Nested tabs get their own scope.
export function TabLoadingScope({ children }) {
  const [statuses, setStatuses] = useState({});
  const register = useCallback((id, label) => setStatuses((previous) => {
    if ((previous[id] ?? null) === label) return previous;
    const next = { ...previous };
    if (label) next[id] = label;
    else delete next[id];
    return next;
  }), []);
  const context = useMemo(() => ({ statuses, register }), [statuses, register]);
  return <TabLoadingContext.Provider value={context}>{children}</TabLoadingContext.Provider>;
}

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

export default function WorkspaceTabs({ id, label, value, onChange, tabs, level = "section", panelId, loading = false, loadingLabel = "Updating…" }) {
  const primary = level === "primary";
  const scope = useContext(TabLoadingContext);
  const labels = [...new Set([...(loading ? [loadingLabel] : []), ...Object.values(scope?.statuses ?? {})])];
  return <Box data-tab-row={id} sx={{ display: "grid", gridTemplateColumns: "minmax(24px, 1fr) minmax(0, max-content) minmax(24px, 1fr)", alignItems: "center", mb: 2 }}>
    <Box data-loading-slot={id} sx={{ display: "flex", justifyContent: "flex-start", overflow: "hidden", minWidth: 0 }}><LoadingStatus compact loading={labels.length > 0} label={labels.join(" ")} /></Box>
    <Tabs value={value} onChange={(_, next) => onChange(next)} aria-label={label}
    variant="scrollable" scrollButtons="auto" allowScrollButtonsMobile
    sx={{ minHeight: primary ? 56 : 48, minWidth: 0, maxWidth: "100%",
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
  </Tabs><Box aria-hidden="true" /></Box>;
}
