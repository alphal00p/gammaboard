import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const normalizeBase = (value) => {
  const raw = value && value.trim() ? value.trim() : "/";
  if (raw === "." || raw === "./") return "./";
  const withLeadingSlash = raw.startsWith("/") ? raw : `/${raw}`;
  return withLeadingSlash.endsWith("/") ? withLeadingSlash : `${withLeadingSlash}/`;
};

export default defineConfig({
  base: normalizeBase(process.env.GAMMABOARD_FRONTEND_BASE),
  plugins: [react()],
  build: {
    outDir: "build",
    // ECharts is lazy-loaded and intentionally lives in a large async chunk.
    chunkSizeWarningLimit: 700,
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/setupTests.js",
    globals: true,
    include: ["src/**/*.{test,spec}.{js,jsx}"],
  },
  server: {
    proxy: {
      "/api": "http://127.0.0.1:4000",
    },
  },
});
