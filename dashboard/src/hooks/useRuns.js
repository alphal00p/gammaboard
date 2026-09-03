import { useCallback, useEffect, useMemo, useState } from "react";
import { fetchRuns } from "../services/api";
import { usePolledResource } from "./usePolledResource";

export const useRuns = ({ refreshInterval = 2000, includeChildren = false } = {}) => {
  const fetchResource = useCallback((signal) => fetchRuns({ includeChildren }, signal), [includeChildren]);
  const { data: firstPage, isConnected } = usePolledResource({
    pollMs: refreshInterval,
    initialData: { items: [], nextOffset: null },
    fetchResource,
    onError: (err) => console.error("Failed to fetch runs:", err),
  });
  const [extraPages, setExtraPages] = useState([]);
  const [isLoadingMore, setIsLoadingMore] = useState(false);

  useEffect(() => {
    setExtraPages([]);
  }, [includeChildren]);

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
    if (nextOffset == null || isLoadingMore) return;
    setIsLoadingMore(true);
    try {
      const page = await fetchRuns({ includeChildren, offset: nextOffset });
      setExtraPages((current) => [...current, page]);
    } catch (err) {
      console.error("Failed to fetch more runs:", err);
    } finally {
      setIsLoadingMore(false);
    }
  }, [includeChildren, isLoadingMore, nextOffset]);

  return { runs, isConnected, hasMoreRuns: nextOffset != null, loadMoreRuns: loadMore, isLoadingMoreRuns: isLoadingMore };
};
