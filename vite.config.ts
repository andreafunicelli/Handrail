import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  root: "ui",
  plugins: [svelte()],
  clearScreen: false,
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: "es2022",
    rollupOptions: { output: { manualChunks: { xterm: ["@xterm/xterm", "@xterm/addon-fit"] } } },
  },
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/target/**", "**/crates/**"] },
  },
  test: { root: ".", include: ["ui/src/**/*.test.ts"] },
});
