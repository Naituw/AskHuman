import { resolve } from "node:path";
import { defineConfig } from "vite";
import { preserveMarkedLookbehindDetection } from "./scripts/vite-mermaid";

// A classic script works from file:// without module CORS or an HTTP server.
export default defineConfig({
  plugins: [preserveMarkedLookbehindDetection],
  build: {
    target: ["es2019", "safari13"],
    outDir: "dist",
    emptyOutDir: false,
    lib: {
      entry: resolve("src/attachmentBrowser.ts"),
      name: "AskHumanAttachmentMermaid",
      formats: ["iife"],
      fileName: () => "attachment-mermaid.js",
    },
  },
});
