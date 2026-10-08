import { describe, expect, it, vi } from "vitest";
import { resolveMarkdownLink } from "./markdownLinks";

vi.mock("./ipc", () => ({ openPath: vi.fn(async () => {}) }));

describe("authored Markdown link destinations", () => {
  it.each([
    ["/Users/test/report.md", "", "/Users/test/report.md"],
    ["/Users/test/report.md:82", "", "/Users/test/report.md"],
    ["/Users/test/report.md:82:5", "", "/Users/test/report.md"],
    ["/Users/test/report.md#L82", "", "/Users/test/report.md"],
    ["/Users/test/My%20Report%20%E4%B8%AD%E6%96%87.md:8", "", "/Users/test/My Report 中文.md"],
    ["/Users/test/name%3A82", "", "/Users/test/name:82"],
    ["docs/report.md:8", "/project", "/project/docs/report.md"],
    ["../report.md", "/project/docs/", "/project/docs/../report.md"],
    ["C:/project/report.md:8:2", "", "C:/project/report.md"],
    ["C:%5Cproject%5Creport.md:8", "", "C:\\project\\report.md"],
    ["docs/report.md", "C:\\project", "C:\\project\\docs/report.md"],
    ["\\\\server\\share\\report.md:8", "", "\\\\server\\share\\report.md"],
    ["file:///Users/test/My%20Report.md:8", "", "/Users/test/My Report.md"],
    ["file:///C:/project/report.md#L8", "", "C:/project/report.md"],
    ["file://server/share/report.md", "", "\\\\server\\share\\report.md"],
    ["https://example.com/report:82#L8", "", "https://example.com/report:82#L8"],
    ["mailto:test@example.com", "", "mailto:test@example.com"],
    ["//example.com/report", "", "https://example.com/report"],
  ])("resolves %s using only its authored path and project", (href, baseDirectory, target) => {
    expect(resolveMarkdownLink(href, baseDirectory)).toEqual({ kind: "open", target });
  });

  it.each(["", "docs/report.md", "?view=settings", "tauri://localhost/index.html", "ftp://example.com/report", "javascript:alert(1)", "data:text/html,test", "/file%XX", "/file%00"])("does not open unsupported destination %s", href => {
    expect(resolveMarkdownLink(href)).toBeNull();
  });

  it("resolves an in-document fragment without opening the file", () => {
    expect(resolveMarkdownLink("#%E4%B8%AD%E6%96%87")).toEqual({ kind: "fragment", target: "中文" });
  });
});
