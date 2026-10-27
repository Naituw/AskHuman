import { configDefaults, defineConfig } from "vitest/config";
import vue from "@vitejs/plugin-vue";

export default defineConfig({
  plugins: [vue()],
  test: {
    environment: "jsdom",
    // relay/ and ios/ have their own runners (`node --test`, `swift test`).
    exclude: [...configDefaults.exclude, "relay/**", "ios/**"],
  },
});
