import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
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
