import { resolve } from "node:path";
import { defineConfig } from "vite";

// The annotation engine (src/lib/annotations.ts) as a classic script for the iOS Markdown page, which has no
// module loader and runs from file://. The engine imports nothing, so the bundle is that one file under the
// global `AHAnnotations`. Built by ios/Tools/web-markdown/build-annotations.sh, which adds the header
// that src/lib/annotations.bundle.test.ts checks.
export default defineConfig({
  build: {
    target: ["es2019", "safari13"],
    // The script passes its own --outDir; this one keeps a plain `vite build` out of the web app's dist/.
    outDir: "dist/ios-annotations",
    emptyOutDir: true,
    // Left readable: the bundle is checked in and debugged in Safari's inspector.
    minify: false,
    lib: {
      entry: resolve("src/lib/annotations.ts"),
      name: "AHAnnotations",
      formats: ["iife"],
      fileName: () => "annotations.js",
    },
  },
});
