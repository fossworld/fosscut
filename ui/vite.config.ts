import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";

// The desktop app loads the dev server at this port (apps/desktop/tauri.conf.json, devUrl).
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist" },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
