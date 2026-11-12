import { useCallback, useEffect, useRef, useState } from "react";
import { fetchRuns } from "../services/api";

// Search is independent of the polled run list and never changes the selection.
export function useRunSearch(query, includeChildren, enabled) {
  const search = query.trim();
  const source = JSON.stringify([search, includeChildren]);
  const controller = useRef(null);
  const [result, setResult] = useState(null);
  useEffect(() => {
    if (!enabled || !search) return undefined;
    const request = new AbortController();
    controller.current = request;
    setResult(null);
    const timer = setTimeout(async () => {
      try {
        const page = await fetchRuns({ search, includeChildren }, request.signal);
        if (!request.signal.aborted) setResult({ ...page, source, loading: false });
      } catch (error) {
        if (!request.signal.aborted) setResult({ source, items: [], error, loading: false });
      }
    }, 200);
    return () => { clearTimeout(timer); request.abort(); };
  }, [search, includeChildren, enabled, source]);

  const current = result?.source === source ? result : null;
  const loadMore = useCallback(async () => {
    const request = controller.current;
    if (!enabled || !request || request.signal.aborted || current?.nextOffset == null || current.loading) return;
    setResult((previous) => ({ ...previous, loading: true, error: null }));
    try {
      const page = await fetchRuns({ search, includeChildren, offset: current.nextOffset }, request.signal);
      if (!request.signal.aborted) setResult((previous) => ({ ...page, source, loading: false,
        items: [...new Map([...previous.items, ...page.items].map((run) => [run.run_id, run])).values()] }));
    } catch (error) {
      if (!request.signal.aborted) setResult((previous) => ({ ...previous, error, loading: false }));
    }
  }, [current, enabled, includeChildren, search, source]);
  return { items: current?.items ?? [], loading: enabled && Boolean(search) && (!current || current.loading),
    error: current?.error, hasMore: current?.nextOffset != null, loadMore };
}
