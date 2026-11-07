// @vitest-environment node
import { describe, expect, it } from "vitest";
import bundle from "../../ios/AskHuman/Resources/WebMarkdown/annotations.js?raw";
import source from "./annotations.ts?raw";

// The iOS Markdown page runs the popup's annotation engine as a checked-in script (annotations.js, built from
// src/lib/annotations.ts by ios/Tools/web-markdown/build-annotations.sh). A script that no longer matches its
// source would make the two platforms find, number and draw passages differently without any other test noticing.

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, "0")).join("");
}

describe("the engine bundle of the iOS page", () => {
  it("is built from the current src/lib/annotations.ts", async () => {
    const header = /^\/\/ Generated from src\/lib\/annotations\.ts \(sha256 ([0-9a-f]{64})\)/.exec(bundle);
    expect(header, "annotations.js must start with the generated-from line").not.toBeNull();
    expect(
      header![1],
      "src/lib/annotations.ts changed: run ios/Tools/web-markdown/build-annotations.sh and commit annotations.js",
    ).toBe(await sha256(source));
  });

  it("defines the one global the page script uses, with what it calls", () => {
    const engine = new Function(`${bundle}\nreturn AHAnnotations;`)() as Record<string, unknown>;
    for (const name of [
      "relocate", "highlight", "markDraft", "marksOf", "flash", "currentAnchor", "overlapping",
      "anchorFromRange", "clearHighlights", "textOf",
    ]) {
      expect(typeof engine[name], name).toBe("function");
    }
  });
});
