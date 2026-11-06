import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// https://vite.dev/config/
export default defineConfig({
  plugins: [react()],
  // Tauri CLI 会接管终端输出，避免 Vite 清屏打断日志
  clearScreen: false,
  envPrefix: ["VITE_", "TAURI_"],
  server: {
    port: 1420,
    strictPort: true,
    host: "localhost",
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  resolve: {
    alias: {
      "@": "/src",
    },
  },
  build: {
    // WebView2 跟随 Chromium 内核，chrome105 与 Tauri 2 官方模板一致
    target: "chrome105",
    outDir: "dist",
    emptyOutDir: true,
    sourcemap: false,
  },
});
