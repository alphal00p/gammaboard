import { useCallback, useEffect, useRef, useState } from "react";
import { deleteTemplateFile, fetchTemplateFile, fetchTemplateList, saveTemplateFile } from "../services/api";

export const useTemplates = ({ kind, enabled = true, onError = null }) => {
  const [templates, setTemplates] = useState([]);
  const mountedRef = useRef(false);
  const onErrorRef = useRef(onError);

  useEffect(() => {
    onErrorRef.current = onError;
  }, [onError]);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
    };
  }, []);

  const reload = useCallback(
    async (signal) => {
      try {
        const items = await fetchTemplateList(kind, signal);
        if (mountedRef.current && !signal?.aborted) setTemplates(items);
        return items;
      } catch (error) {
        if (error?.name !== "AbortError" && mountedRef.current) onErrorRef.current?.(error);
        return null;
      }
    },
    [kind],
  );

  useEffect(() => {
    if (!enabled) return undefined;
    const controller = new AbortController();
    reload(controller.signal);
    return () => controller.abort();
  }, [enabled, reload]);

  const load = useCallback(
    async (name, signal) => {
      const response = await fetchTemplateFile(kind, name, signal);
      return response?.toml || "";
    },
    [kind],
  );

  const save = useCallback(
    async (name, toml, signal) => {
      const response = await saveTemplateFile(kind, { name, toml }, signal);
      const savedName = String(response?.name || name).trim();
      if (savedName && mountedRef.current) {
        setTemplates((current) =>
          Array.from(new Set([...current.filter((entry) => entry !== name), savedName])).sort((left, right) =>
            left.localeCompare(right),
          ),
        );
      }
      return response;
    },
    [kind],
  );

  const remove = useCallback(
    async (name, signal) => {
      const response = await deleteTemplateFile(kind, name, signal);
      if (mountedRef.current) setTemplates((current) => current.filter((entry) => entry !== name));
      return response;
    },
    [kind],
  );

  return { templates, load, save, remove };
};
