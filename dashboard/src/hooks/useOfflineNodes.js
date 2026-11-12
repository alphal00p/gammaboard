import { useRef, useState } from "react";
import { useAuth } from "../auth/AuthProvider";
import { usePolledResource } from "./usePolledResource";
import { fetchOfflineNodes, updateOfflineNode } from "../services/api";
import { compareNodesByName } from "../utils/nodes";

export const useOfflineNodes = ({ enabled = true } = {}) => {
  const { authenticated, allowLocalNodeSpawn = true } = useAuth();
  const resource = usePolledResource({ paused: !enabled, pollMs: 3000, initialData: [], fetchResource: fetchOfflineNodes });
  const { data, error, isLoading, refresh } = resource;
  const [busy, setBusy] = useState(null);
  const busyRef = useRef(false);
  const [message, setMessage] = useState(null);
  const [pending, setPending] = useState(new Set());
  const [previousData, setPreviousData] = useState(data);
  if (data !== previousData) { setPreviousData(data); setPending(new Set()); }
  const nodes = Array.isArray(data) ? [...data].sort(compareNodesByName) : [];
  const identity = (node) => `${node.node_name}:${node.node_uuid}`;
  const disabledReason = (node, action) => {
    if (!authenticated) return "Enable operator mode to manage workers.";
    if (busy || pending.has(identity(node))) return "Waiting for the worker update.";
    if (isLoading || error) return "Waiting for a fresh worker list.";
    if (node.launch_pending) return "This worker is awaiting registration for a queued launch.";
    if (action === "resume") {
      if (!node.can_resume) return "No saved launch settings; start this worker through its launcher.";
      if (node.backend === "local" && !allowLocalNodeSpawn) return "Local worker launch is disabled on this server.";
    }
    return null;
  };
  const eligible = (action) => nodes.filter((node) => !disabledReason(node, action));
  const act = async (action, node = null) => {
    if (busyRef.current) return;
    const targets = node ? (disabledReason(node, action) ? [] : [node]) : eligible(action);
    if (!targets.length) return;
    busyRef.current = true;
    setMessage(null);
    let succeeded = 0;
    const failures = [];
    try {
      // Every request validates the captured identity and current lease. A bulk
      // action never silently targets new workers arriving during the operation.
      for (const [index, target] of targets.entries()) {
        setBusy(`${action === "resume" ? "Resuming" : action === "retire" ? "Retiring" : "Releasing"} workers: ${index + 1}/${targets.length}…`);
        try {
          await updateOfflineNode(target, action);
          succeeded += 1;
          setPending((previous) => new Set([...previous, identity(target)]));
        } catch (err) {
          failures.push(`${target.node_name}: ${err?.message || "Request failed"}`);
        }
      }
      const verb = action === "resume" ? "Queued resume for" : action === "retire" ? "Retired" : "Released assignments for";
      const skipped = node ? 0 : nodes.length - targets.length;
      setMessage({
        severity: failures.length ? "warning" : "success",
        text: `${verb} ${node && succeeded ? node.node_name : `${succeeded} worker${succeeded === 1 ? "" : "s"}`}.${skipped ? ` Skipped ${skipped} unavailable worker${skipped === 1 ? "" : "s"}.` : ""}${failures.length ? ` Failed: ${failures.join("; ")}` : ""}`,
      });
    } finally {
      busyRef.current = false;
      setBusy(null);
      refresh();
    }
  };
  return { ...resource, nodes, authenticated, busy, message, disabledReason, eligible, act };
};
