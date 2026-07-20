import { describe, expect, it } from "vitest";
import { settleTextareaHeightAfterBlur } from "./textareaAutosize";

describe("settleTextareaHeightAfterBlur", () => {
  it("does not mutate the measured height of an expanded editor", async () => {
    const textarea = document.createElement("textarea");
    textarea.style.height = "62px";
    const mutations: MutationRecord[] = [];
    const observer = new MutationObserver((records) => mutations.push(...records));
    observer.observe(textarea, { attributes: true, attributeFilter: ["style"] });

    settleTextareaHeightAfterBlur(textarea, true);
    await Promise.resolve();

    expect(textarea.style.height).toBe("62px");
    expect(mutations).toHaveLength(0);
    observer.disconnect();
  });

  it("clears the measured height when the editor collapses", () => {
    const textarea = document.createElement("textarea");
    textarea.style.height = "62px";

    settleTextareaHeightAfterBlur(textarea, false);

    expect(textarea.style.height).toBe("");
  });
});
