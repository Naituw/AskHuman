import type { PluginOption } from "vite";

// Keep Marked's lookbehind feature probe dynamic for Safari 13 in both bundles.
export const preserveMarkedLookbehindDetection: PluginOption = {
  name: "preserve-marked-lookbehind-detection",
  enforce: "pre",
  transform(code, id) {
    if (!id.includes("/marked") || !id.endsWith("/lib/marked.esm.js")) return;
    const probe = 'new RegExp("(?<=1)(?<!1)")';
    if (!code.includes(probe)) return;
    return code.replace(
      probe,
      'new RegExp("(?" + String.fromCharCode(60) + "=1)(?" + String.fromCharCode(60) + "!1)")',
    );
  },
};
