import { useLayoutEffect, useRef, useState } from "react";
import { Box } from "@mui/material";
import { useWorkspaceActive } from "./WorkspaceTabs";

// Keep the document tall enough during task replacement, without showing the
// previous task's data under the new selection or forcing scroll restoration.
export default function StableTaskOutput({ loading, source, children }) {
  const outer = useRef(null);
  const content = useRef(null);
  const [minimumHeight, setMinimumHeight] = useState(0);
  const active = useWorkspaceActive();
  useLayoutEffect(() => {
    if (!active || loading) return;
    let frame;
    const measure = () => {
      if (!outer.current || !content.current) return;
      const naturalHeight = content.current.getBoundingClientRect().height;
      // If the new output is shorter, release spare space as the user scrolls
      // upward. No permanent page-sized gap and no jump to a cached scroll Y.
      const visibleHeight = Math.max(0, window.innerHeight - outer.current.getBoundingClientRect().top);
      setMinimumHeight((previous) => Math.ceil(Math.max(naturalHeight, Math.min(previous, visibleHeight))));
    };
    const schedule = () => { cancelAnimationFrame(frame); frame = requestAnimationFrame(measure); };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? null : new ResizeObserver(schedule);
    observer?.observe(content.current);
    window.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      observer?.disconnect(); cancelAnimationFrame(frame);
      window.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
    };
  }, [active, loading, source]);
  return <Box ref={outer} aria-busy={loading || undefined} data-task-output-container
    sx={{ minHeight: minimumHeight, overflowAnchor: "none" }}>
    <Box ref={content} sx={{ display: "flow-root" }}>{children}</Box>
  </Box>;
}
