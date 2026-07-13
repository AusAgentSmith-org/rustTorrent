import { defineConfig } from "vite";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { resolve } from "node:path";
import svgr from "vite-plugin-svgr";

// https://vitejs.dev/config/
export default defineConfig({
  plugins: [react(), svgr(), tailwindcss()],
  server: {
    host: true,
    port: 3031,
  },
  build: {
    manifest: true,
    rollupOptions: process.env.RTBIT_DEMO_BUILD
      ? { input: resolve(__dirname, "mock.html") }
      : undefined,
  },
});
