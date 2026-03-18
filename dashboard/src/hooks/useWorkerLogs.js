import { useCallback, useEffect, useMemo, useState } from "react";
import { fetchRunLogPage } from "../services/api";
import { asArray } from "../utils/collections";

const defaultFilters = Object.freeze({
  nodeId: "",
  level: "",
  search: "",
});

export const useWorkerLogs = ({ runId, workers = [], limit = 100 } = {}) => {
  const [items, setItems] = useState([]);
  const [filters, setFilters] = useState(defaultFilters);
  const [cursor, setCursor] = useState(null);
  const [hasMoreOlder, setHasMoreOlder] = useState(false);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState(null);

  const workerOptions = useMemo(() => {
    const runWorkers = asArray(workers).filter(
      (worker) => runId == null || worker.current_run_id === runId || worker.desired_run_id === runId,
    );
    return runWorkers
      .map((worker) => worker.node_id || worker.worker_id)
      .filter(Boolean)
      .sort((left, right) => left.localeCompare(right));
  }, [workers, runId]);

  useEffect(() => {
    setItems([]);
    setCursor(null);
    setHasMoreOlder(false);
    setError(null);
    setFilters(defaultFilters);
  }, [runId]);

  const loadPage = useCallback(
    async ({ beforeId = null, append = false, signal } = {}) => {
      if (runId == null) {
        setItems([]);
        setCursor(null);
        setHasMoreOlder(false);
        setError(null);
        return;
      }

      setIsLoading(true);
      try {
        const page = await fetchRunLogPage(
          runId,
          {
            limit,
            nodeId: filters.nodeId || null,
            level: filters.level || null,
            search: filters.search || "",
            beforeId,
          },
          signal,
        );
        setItems((previous) => (append ? [...previous, ...page.items] : page.items));
        setCursor(page.next_before_id ?? null);
        setHasMoreOlder(page.has_more_older === true);
        setError(null);
      } catch (err) {
        if (err?.name === "AbortError") return;
        setError(err);
        if (!append) {
          setItems([]);
          setCursor(null);
          setHasMoreOlder(false);
        }
      } finally {
        setIsLoading(false);
      }
    },
    [runId, limit, filters],
  );

  useEffect(() => {
    if (runId == null) return undefined;
    const controller = new AbortController();
    loadPage({ beforeId: null, append: false, signal: controller.signal });
    return () => controller.abort();
  }, [runId, filters, loadPage]);

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
    error,
    refresh,
    loadOlder,
  };
};
