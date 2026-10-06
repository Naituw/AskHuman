import { describe, expect, it } from "vitest";
import type { PopupInboxRequest } from "../../lib/types";
import { inboxGroups, inboxTitle, nextInboxRequest } from "./inboxQueue";
function request(id: string, project: string, sequence: number): PopupInboxRequest {
  return { requestId: id, project, sequence, lang: "en", source: "Codex", createdAtMs: 1,
    interaction: { type: "ask", request: { id, isMarkdown: true, selectOnly: false, single: false, outputFormat: "text",
      message: { text: "context", files: [] }, questions: [{ message: "**Choose**\n a layout", predefinedOptions: [] }] } } };
}
describe("request inbox navigation", () => {
  const a = request("a", "/one/app", 2), b = request("b", "/two/app", 1), c = request("c", "/one/app", 3), d = request("d", "/three", 4);
  it("keeps exact project identities and stable first-arrival group order", () => {
    const groups = inboxGroups([c, a, d, b], [a.project, b.project, d.project]);
    expect(groups.map(g => g.requests.map(r => r.requestId))).toEqual([["a", "c"], ["b"], ["d"]]);
  });
  it("continues within the current project before moving to the next group", () => {
    const ordered = [a, c, b, d];
    expect(nextInboxRequest(ordered, "a", new Set(["a"]))).toBe("c");
    expect(nextInboxRequest(ordered, "c", new Set(["c"]))).toBe("a");
    expect(nextInboxRequest(ordered, "c", new Set(["a", "c"]))).toBe("b");
    expect(nextInboxRequest(ordered, "d", new Set(["d"]))).toBe("a");
  });
  it("does not select a removed request after a cancellation snapshot", () => {
    expect(nextInboxRequest([a, c, b, d], "a", new Set(["a", "c", "b"]))).toBe("d");
    expect(nextInboxRequest([a], "a", new Set(["a"]))).toBeNull();
  });
  it("uses a readable question summary instead of raw Markdown", () => { expect(inboxTitle(a)).toBe("Choose a layout"); });
});
