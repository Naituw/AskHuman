import { defaultInclude, defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  test: {
    environment: "jsdom",
    // Discover frontend tests only, even when build artifacts contain source copies.
    include: defaultInclude.map((pattern) => `src/${pattern}`),
  },
});
