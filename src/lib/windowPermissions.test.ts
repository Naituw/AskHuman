import { describe, expect, it } from "vitest";
import capability from "../../src-tauri/capabilities/default.json";

// A window API the page calls without a matching grant fails at run time with "not allowed by ACL" (the inbox's
// tray-click focus did, until `core:window:allow-set-focus` was added), and nothing else notices: the types are fine.
const sources = import.meta.glob(["../**/*.ts", "../**/*.vue"], { query: "?raw", import: "default", eager: true }) as Record<string, string>;

const CALL = /\bgetCurrent(?:Webview)?Window\(\)\.(\w+)\(/g;
const kebab = (name: string) => name.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`);

describe("window permissions", () => {
  const used = new Map<string, string>();
  for (const [file, text] of Object.entries(sources)) {
    if (file.endsWith(".test.ts") || file.includes("/prototype/")) continue;
    for (const [, method] of text.matchAll(CALL)) used.set(method, file);
  }

  it("finds the calls this check is about", () => {
    expect([...used.keys()]).toEqual(expect.arrayContaining(["setFocus", "setAlwaysOnTop", "close"]));
  });

  it.each([...used])("grants %s to the windows that call it", (method, file) => {
    expect(capability.windows).toEqual(["*"]);
    expect(capability.permissions, `${file} calls ${method}()`).toContain(`core:window:allow-${kebab(method)}`);
  });
});
