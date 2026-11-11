import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { licences } from "./scripts/licences.mjs";

export default defineConfig({
  // licences(): the open-source licences the app ships, read from the lockfiles at each build
  // for Settings (scripts/licences.mjs).
  plugins: [react(), licences()],
  clearScreen: false,
  server: {
    port: 5273,
    strictPort: true,
    // Only the page's own sources are watched. The Rust build folders (target*, hundreds of
    // thousands of files) and packaging output ran the system out of file watchers, and the
    // dev server would not start.
    watch: { ignored: ["**/target/**", "**/target-*/**", "**/packaging/**", "**/src-tauri/**", "**/crates/**", "**/.uiverify/**", "**/docs/**"] },
  },
  build: { target: "safari15", sourcemap: false },
});
