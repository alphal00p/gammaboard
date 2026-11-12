import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { fetchRuns, fetchRunSummary } from "../services/api";
import { usePolledResource } from "./usePolledResource";

const useRunCatalog = ({ refreshInterval, includeChildren, paused = false }) => {
  const fetchResource = useCallback((signal) => fetchRuns({ includeChildren }, signal), [includeChildren]);
  const { data: firstPage, isConnected, isLoading, isFetching } = usePolledResource({
    pollMs: refreshInterval,
    paused,
    initialData: { items: [], nextOffset: null },
    fetchResource,
    onError: (err) => console.error("Failed to fetch runs:", err),
  });
  const [extraPages, setExtraPages] = useState([]);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const pageRequest = useRef(null);

  useEffect(() => {
    if (paused) setIsLoadingMore(false);
    return () => { pageRequest.current?.abort(); pageRequest.current = null; };
  }, [paused]);

  const pages = useMemo(() => [firstPage, ...extraPages], [firstPage, extraPages]);
  const runs = useMemo(() => {
    const seen = new Set();
    return pages.flatMap((page) => page.items).filter((run) => {
      if (seen.has(run.run_id)) return false;
      seen.add(run.run_id);
      return true;
    });
  }, [pages]);
  const nextOffset = pages.at(-1)?.nextOffset ?? null;
  const loadMore = useCallback(async () => {
    if (paused || nextOffset == null || pageRequest.current) return;
    const request = new AbortController();
    pageRequest.current = request;
    setIsLoadingMore(true);
    try {
      const page = await fetchRuns({ includeChildren, offset: nextOffset }, request.signal);
      if (!request.signal.aborted) setExtraPages((current) => [...current, page]);
    } catch (err) {
      if (!request.signal.aborted) console.error("Failed to fetch more runs:", err);
    } finally {
      if (pageRequest.current === request) { pageRequest.current = null; setIsLoadingMore(false); }
    }
  }, [includeChildren, paused, nextOffset]);

  return { runs, isConnected, isLoadingRuns: isLoading, isFetchingRuns: isFetching, hasMoreRuns: nextOffset != null, loadMoreRuns: loadMore, isLoadingMoreRuns: isLoadingMore };
};

export const useRuns = ({ refreshInterval = 2000, includeChildren = false, selectedRun = null } = {}) => {
  // The root catalog owns selection and pagination. Showing children augments
  // it instead of replacing it with a temporarily empty request result.
  const roots = useRunCatalog({ refreshInterval, includeChildren: false });
  const children = useRunCatalog({ refreshInterval, includeChildren: true, paused: !includeChildren });
  const catalogRuns = useMemo(() => includeChildren
    ? [...roots.runs, ...children.runs.filter((run) => run.parent_run_id != null)] : roots.runs,
  [roots.runs, children.runs, includeChildren]);
  const catalogLoading = roots.isLoadingRuns || (includeChildren && children.isLoadingRuns);
  const needsSelectedRun = selectedRun != null && !roots.isLoadingRuns && !catalogRuns.some((run) => run.run_id === selectedRun);
  const fetchSelected = useCallback((signal) => fetchRunSummary(selectedRun, signal), [selectedRun]);
  const selected = usePolledResource({ enabled: needsSelectedRun, pollMs: refreshInterval,
    initialData: null, fetchResource: fetchSelected });
  const selectedVisible = selected.data && (includeChildren || selected.data.parent_run_id == null);
  const selectedRunMissing = Boolean(needsSelectedRun && (selected.error?.status === 404 || (selected.data && !selectedVisible)));
  const runs = useMemo(() => needsSelectedRun && selectedVisible
    ? [...catalogRuns, selected.data] : catalogRuns, [catalogRuns, needsSelectedRun, selectedVisible, selected.data]);
  const loadMoreRuns = useCallback(() => Promise.all([
    roots.loadMoreRuns(), ...(includeChildren ? [children.loadMoreRuns()] : []),
  ]), [roots.loadMoreRuns, children.loadMoreRuns, includeChildren]);
  return {
    runs, loadMoreRuns, selectedRunMissing,
    isConnected: roots.isConnected && (!includeChildren || children.isConnected),
    isLoadingRuns: catalogLoading || (needsSelectedRun && selected.isLoading),
    isFetchingRuns: roots.isFetchingRuns || (includeChildren && children.isFetchingRuns),
    hasMoreRuns: roots.hasMoreRuns || (includeChildren && children.hasMoreRuns),
    isLoadingMoreRuns: roots.isLoadingMoreRuns || (includeChildren && children.isLoadingMoreRuns),
  };
};
