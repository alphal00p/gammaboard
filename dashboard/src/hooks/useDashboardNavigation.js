import { useCallback, useEffect, useRef, useState } from "react";

export const DEFAULT_NAVIGATION = Object.freeze({
  workspace: "runs", runTab: "tasks", taskTab: "result", performanceTab: "overview",
  systemTab: "workers", runId: null, taskId: null, children: false,
});
const choices = {
  workspace: ["runs", "system", "logs"], runTab: ["tasks", "summary", "performance", "settings"],
  taskTab: ["result", "observables", "strata", "diagnostics"],
  performanceTab: ["overview", "diagnostics", "graphs", "tuning"], systemTab: ["workers", "database", "settings"],
};
export function readNavigation(hash = window.location.hash) {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const next = { ...DEFAULT_NAVIGATION };
  for (const [key, allowed] of Object.entries(choices)) if (allowed.includes(params.get(key))) next[key] = params.get(key);
  const run = Number(params.get("run"));
  next.runId = Number.isSafeInteger(run) && run > 0 ? run : null;
  // Task IDs are bigint strings; never round them through Number.
  next.taskId = /^[1-9][0-9]*$/.test(params.get("task") || "") ? params.get("task") : null;
  next.children = params.get("children") === "1";
  return next;
}
export function navigationHash(state) {
  const params = new URLSearchParams();
  for (const key of Object.keys(choices)) if (state[key] !== DEFAULT_NAVIGATION[key]) params.set(key, state[key]);
  if (state.runId != null) params.set("run", state.runId);
  if (state.taskId != null) params.set("task", state.taskId);
  if (state.children) params.set("children", "1");
  return params.size ? `#${params}` : "";
}
export function useDashboardNavigation() {
  const [navigation, setNavigation] = useState(readNavigation);
  const current = useRef(navigation);
  const navigate = useCallback((patch, { replace = false } = {}) => {
    const next = { ...current.current, ...patch };
    const hash = navigationHash(next);
    if (hash === navigationHash(current.current)) return;
    if (hash !== window.location.hash) window.history[replace ? "replaceState" : "pushState"](null, "", `${window.location.pathname}${window.location.search}${hash}`);
    current.current = next;
    setNavigation(next);
  }, []);
  useEffect(() => {
    const restore = () => { current.current = readNavigation(); setNavigation(current.current); };
    window.addEventListener("popstate", restore);
    window.addEventListener("hashchange", restore);
    return () => { window.removeEventListener("popstate", restore); window.removeEventListener("hashchange", restore); };
  }, []);
  return [navigation, navigate];
}
