import { useCallback, useEffect, useMemo, useState } from "react";
import { fetchRuntimeLogPage } from "../services/api";
import { useWorkspaceActive } from "../components/common/WorkspaceTabs";
import { asArray } from "../utils/collections";

const defaultFilters = Object.freeze({
  runId: "",
  includeChildren: false,
  source: "",
  nodeName: "",
  level: "",
  search: "",
});

export const useWorkerLogs = ({ runId = null, workers = [], limit = 100 } = {}) => {
  const active = useWorkspaceActive();
  const [items, setItems] = useState([]);
  const [filters, setFilters] = useState(() => ({
    ...defaultFilters,
    runId: runId == null ? "" : String(runId),
  }));
  const [cursor, setCursor] = useState(null);
  const [hasMoreOlder, setHasMoreOlder] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [loadedFilters, setLoadedFilters] = useState(null);
  const [error, setError] = useState(null);

  const workerOptions = useMemo(() => {
    const selectedRunId = filters.runId === "" ? null : Number(filters.runId);
    const runWorkers = asArray(workers).filter(
      (worker) =>
        selectedRunId == null ||
        worker.current_run_id === selectedRunId ||
        worker.desired_run_id === selectedRunId,
    );
    const options = new Set(runWorkers.map((worker) => worker.node_name).filter(Boolean));
    for (const item of asArray(items)) {
      if (item?.node_name) options.add(item.node_name);
    }
    if (filters.nodeName) options.add(filters.nodeName);
    return Array.from(options).sort((left, right) => left.localeCompare(right));
  }, [workers, items, filters.runId, filters.nodeName]);

  useEffect(() => {
    setItems([]);
    setCursor(null);
    setHasMoreOlder(false);
    setError(null);
    setFilters({
      ...defaultFilters,
      runId: runId == null ? "" : String(runId),
    });
  }, [runId]);

  const loadPage = useCallback(
    async ({ beforeId = null, append = false, signal } = {}) => {
      setIsLoading(true);
      try {
        const selectedRunId = filters.runId === "" ? null : Number(filters.runId);
        const page = await fetchRuntimeLogPage(
          {
            limit,
            source: filters.source || null,
            runId: Number.isFinite(selectedRunId) ? selectedRunId : null,
            includeChildren: filters.includeChildren === true,
            nodeName: filters.nodeName || null,
            nodeUuid: null,
            level: filters.level || null,
            search: filters.search || "",
            beforeId,
          },
          signal,
        );
        if (signal?.aborted) return;
        setItems((previous) => (append ? [...previous, ...page.items] : page.items));
        setCursor(page.next_before_id ?? null);
        setHasMoreOlder(page.has_more_older === true);
        setError(null);
      } catch (err) {
        if (signal?.aborted || err?.name === "AbortError") return;
        setError(err);
        if (!append) {
          setItems([]);
          setCursor(null);
          setHasMoreOlder(false);
        }
      } finally {
        if (!signal?.aborted) {
          setLoadedFilters(filters);
          setIsLoading(false);
        }
      }
    },
    [limit, filters],
  );

  useEffect(() => {
    if (!active) return;
    const controller = new AbortController();
    loadPage({ beforeId: null, append: false, signal: controller.signal });
    return () => controller.abort();
  }, [active, filters, loadPage]);

  const refresh = useCallback(() => {
    const controller = new AbortController();
    loadPage({ beforeId: null, append: false, signal: controller.signal });
  }, [loadPage]);

  const loadOlder = useCallback(() => {
    if (!cursor || isLoading) return;
    const controller = new AbortController();
    loadPage({ beforeId: cursor, append: true, signal: controller.signal });
  }, [cursor, isLoading, loadPage]);

  return {
    items,
    filters,
    setFilters,
    workerOptions,
    hasMoreOlder,
    isLoading,
    isInitialLoading: loadedFilters !== filters,
    error,
    refresh,
    loadOlder,
  };
};
