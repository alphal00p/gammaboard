import { useEffect, useMemo, useRef, useState } from "react";
import { Box, Typography } from "@mui/material";

const MATHJAX_CDN = "https://cdn.jsdelivr.net/npm/mathjax@3/es5/tex-mml-chtml.js";
let mathJaxLoadPromise;

const ensureMathJaxLoaded = () => {
  if (typeof window === "undefined") {
    return Promise.reject(new Error("MathJax unavailable outside browser"));
  }

  if (window.MathJax?.typesetPromise) {
    return Promise.resolve(window.MathJax);
  }

  if (mathJaxLoadPromise) {
    return mathJaxLoadPromise;
  }

  window.MathJax = window.MathJax || {
    tex: {
      inlineMath: [
        ["\\(", "\\)"],
        ["$", "$"],
      ],
      displayMath: [
        ["\\[", "\\]"],
        ["$$", "$$"],
      ],
    },
    options: {
      skipHtmlTags: ["script", "noscript", "style", "textarea", "pre", "code"],
    },
  };

  mathJaxLoadPromise = new Promise((resolve, reject) => {
    let settled = false;
    const timeout = window.setTimeout(() => {
      if (settled) return;
      settled = true;
      reject(new Error("Timed out while loading MathJax"));
    }, 8000);
    const finishResolve = () => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      resolve(window.MathJax);
    };
    const finishReject = () => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      reject(new Error("Failed to load MathJax"));
    };

    const existing = document.querySelector(`script[src="${MATHJAX_CDN}"]`);
    if (existing) {
      existing.addEventListener("load", finishResolve, { once: true });
      existing.addEventListener("error", finishReject, { once: true });
      return;
    }

    const script = document.createElement("script");
    script.src = MATHJAX_CDN;
    script.async = true;
    script.onload = finishResolve;
    script.onerror = finishReject;
    document.head.appendChild(script);
  });

  return mathJaxLoadPromise;
};

const LatexFormula = ({ latex, display = true, fallbackPrefix = "LaTeX" }) => {
  const hostRef = useRef(null);
  const [renderError, setRenderError] = useState(null);

  const mathSource = useMemo(() => {
    if (!latex) return "";
    return display ? `\\[${latex}\\]` : `\\(${latex}\\)`;
  }, [display, latex]);

  useEffect(() => {
    let cancelled = false;

    const render = async () => {
      if (!hostRef.current || !mathSource) return;
      try {
        const mathJax = await ensureMathJaxLoaded();
        if (cancelled || !hostRef.current) return;

        hostRef.current.textContent = mathSource;
        await mathJax.typesetPromise([hostRef.current]);
        if (!cancelled) setRenderError(null);
      } catch (error) {
        if (cancelled) return;
        setRenderError(error instanceof Error ? error.message : "LaTeX render failed");
      }
    };

    render();

    return () => {
      cancelled = true;
    };
  }, [mathSource]);

  if (renderError) {
    return (
      <Box>
        <Typography variant="body2" sx={{ fontFamily: "monospace" }}>
          {fallbackPrefix}: {latex}
        </Typography>
      </Box>
    );
  }

  return <Box ref={hostRef} />;
};

export default LatexFormula;
