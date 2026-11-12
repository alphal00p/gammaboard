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

export default function WorkspaceTabs({ id, label, value, onChange, tabs, disabledTabs = {}, actions, level = "section", panelId, loading = false, loadingLabel = "Updating…" }) {
  const primary = level === "primary";
  const height = primary ? 44 : 36;
  const scope = useContext(TabLoadingContext);
  const labels = [...new Set([...(loading ? [loadingLabel] : []), ...Object.values(scope?.statuses ?? {})])];
  // A retained view can become unavailable (e.g. sampling completes). Keep
  // keyboard entry on an enabled tab without changing the displayed panel.
  const entryKey = tabs.find(([key]) => key === value && !disabledTabs[key])?.[0]
    ?? tabs.find(([key]) => !disabledTabs[key])?.[0];
  return <Box data-tab-row={id} sx={{ display: "grid", gridTemplateColumns: primary
    ? { xs: "minmax(0, 1fr) 24px", sm: "minmax(24px, 1fr) minmax(0, max-content) minmax(24px, 1fr)" }
    : actions ? { xs: "minmax(0, 1fr) 24px", md: "minmax(0, 1fr) 24px auto" } : "minmax(0, 1fr) 24px",
    alignItems: "center", mb: 1, gap: actions ? 0.5 : 0,
    borderBottom: level === "section" ? 1 : 0, borderColor: "divider" }}>
    <Box data-loading-slot={id} sx={{ gridRow: 1, gridColumn: primary ? { xs: 2, sm: 1 } : 2,
      display: "flex", overflow: "hidden", minWidth: 0 }}><LoadingStatus compact loading={labels.length > 0} label={labels.join(" ")} /></Box>
    <Tabs value={value} onChange={(_, next) => onChange(next)} aria-label={label}
    variant="scrollable" scrollButtons="auto" allowScrollButtonsMobile
    sx={{ gridRow: 1, gridColumn: primary ? { xs: 1, sm: 2 } : 1, minHeight: height, minWidth: 0, maxWidth: "100%",
      "& .MuiTabs-flexContainer": { width: "max-content", minWidth: "100%", justifyContent: primary ? { xs: "flex-start", sm: "center" } : "flex-start", gap: primary ? 1 : 0 },
      "& .MuiTabs-indicator": { display: level === "section" ? "block" : "none" },
      "& .MuiTab-root": { minWidth: 0, minHeight: height, py: 0.75, px: { xs: 1.25, sm: 2 }, textTransform: "none",
        fontSize: primary ? "1rem" : "0.875rem", borderRadius: level === "section" ? 0 : 1,
        "&.Mui-disabled": { pointerEvents: "auto", cursor: "default" },
        "&.Mui-focusVisible": { outline: "2px solid", outlineColor: "primary.main", outlineOffset: -2 },
        "&.Mui-selected": primary ? { bgcolor: "primary.main", color: "primary.contrastText" }
          : level === "detail" ? { bgcolor: "action.selected", color: "text.primary" } : {},
        "&.Mui-selected.Mui-disabled": { color: "text.disabled", bgcolor: "action.hover" },
      },
    }}>
    {tabs.map(([key, text]) => <Tab key={key} value={key} label={text} disabled={Boolean(disabledTabs[key])} title={disabledTabs[key] || undefined}
      tabIndex={key === entryKey ? 0 : -1}
      id={`${id}-tab-${key}`} aria-controls={panelId || `${id}-panel-${key}`} />)}
  </Tabs>{actions && <Box sx={{ gridColumn: { xs: "1 / -1", md: 3 }, justifySelf: "end" }}>{actions}</Box>}</Box>;
}
