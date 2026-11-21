import { beforeAll, describe, expect, it, vi } from "vitest";
import { handleCodeCopyClick, markdownReady, renderMarkdown } from "./markdown";

// The renderer is loaded lazily (R4 bundle split); tests exercise the real one.
beforeAll(() => markdownReady);

describe("renderMarkdown", () => {
  it("escapes raw HTML (html: false)", () => {
    const html = renderMarkdown('<img src=x onerror=alert(1)> **bold**');
    expect(html).not.toContain("<img");
    expect(html).toContain("&lt;img");
    expect(html).toContain("<strong>bold</strong>");
  });

  it("linkifies bare URLs", () => {
    expect(renderMarkdown("see https://example.com")).toContain(
      '<a href="https://example.com">'
    );
  });

  it("keeps local image paths out of the app resource origin when native loading is enabled", () => {
    const html = renderMarkdown("![preview](/tmp/My%20Preview.png)", { localImages: true });
    expect(html).toContain('data-local-image="/tmp/My Preview.png"');
    expect(html).not.toContain("src=");
    expect(html).toContain('alt="preview"');
    expect(renderMarkdown("![remote](https://example.com/image.png)", { localImages: true })).toContain('src="https://example.com/image.png"');
    expect(renderMarkdown("![unsafe](javascript:alert(1))", { localImages: true })).not.toContain("<img");
    expect(renderMarkdown("![unsafe](data:text/html,test)", { localImages: true })).not.toContain("<img");
  });

  it("marks a link to a local file with the path a click names, for hosts that open local files", () => {
    const html = renderMarkdown(
      "[a](docs/a.md:12) [b](/tmp/b%20c.md \"Mine\") [c](file:///tmp/c.md#L3) [d](C:/x/d.md)",
      { localImages: true, baseDirectory: "/project" },
    );
    expect(html).toContain('data-local-file="/project/docs/a.md"');
    expect(html).toContain('data-local-file="/tmp/b c.md"');
    expect(html).toContain('data-local-file="/tmp/c.md"');
    expect(html).toContain('data-local-file="C:/x/d.md"');
    // Hover names the file; a title the author wrote is kept.
    expect(html).toContain('title="/project/docs/a.md"');
    expect(html).toContain('title="Mine"');
    expect(html).not.toContain('title="/tmp/b c.md"');
    // The authored href stays what the WebView-blocking handlers read.
    expect(html).toContain('href="docs/a.md:12"');
  });

  it("marks no other link, and nothing at all when the host does not open local files", () => {
    const options = { localImages: true, baseDirectory: "/project" };
    const others = renderMarkdown(
      "[w](https://example.com/a.md) [m](mailto:x@y.z) [h](#section) https://example.com [j](javascript:alert(1)) [t](tel:+123)",
      options,
    );
    expect(others).not.toContain("data-local-file");
    // A relative link has no project to be relative to, so it names no file.
    expect(renderMarkdown("[a](docs/a.md)", { localImages: true })).not.toContain("data-local-file");
    // Console, update notes and confirm cards render without the option and keep their links as they were.
    expect(renderMarkdown("[a](/tmp/a.md)")).not.toContain("data-local-file");
    expect(renderMarkdown("[a](/tmp/a.md)", { baseDirectory: "/project" })).not.toContain("data-local-file");
  });

  it("escapes the path it writes into the markup", () => {
    const html = renderMarkdown("[x](</tmp/a&b\"c.md>)", { localImages: true });
    expect(html).toContain('data-local-file="/tmp/a&amp;b&quot;c.md"');
    expect(html).not.toContain('b"c.md');
  });

  it("wraps fenced code blocks with a localized copy button", () => {
    const html = renderMarkdown("```\nlet x = 1;\n```", {
      copyLabel: "复制",
      copiedLabel: "已复制",
    });
    expect(html).toContain('<div class="code-block">');
    expect(html).toContain('data-copy="复制"');
    expect(html).toContain('data-copied="已复制"');
  });

  it("escapes HTML in the copy labels", () => {
    const html = renderMarkdown("```\nx\n```", { copyLabel: '"><script>' });
    expect(html).not.toContain('data-copy=""><script>');
  });

  it("marks Mermaid fences without changing ordinary fences", () => {
    const mermaid = renderMarkdown("``` Mermaid extra\ngraph TD; A-->B\n```");
    const ordinary = renderMarkdown("```typescript\nconst x = 1\n```");

    expect(mermaid).toContain('class="code-block mermaid-block"');
    expect(mermaid).toContain("data-mermaid-pending");
    expect(mermaid).toContain("graph TD; A--&gt;B");
    expect(ordinary).toContain('<div class="code-block">');
    expect(ordinary).not.toContain("data-mermaid-pending");
  });

  it("only treats the first normalized info token as Mermaid", () => {
    expect(renderMarkdown("```not-mermaid mermaid\nx\n```")).not.toContain(
      "data-mermaid-pending",
    );
    expect(renderMarkdown("```mermaidish\nx\n```")).not.toContain(
      "data-mermaid-pending",
    );
  });
});

describe("handleCodeCopyClick", () => {
  function renderIntoDom(markdown: string): HTMLElement {
    const host = document.createElement("div");
    host.innerHTML = renderMarkdown(markdown);
    document.body.appendChild(host);
    return host;
  }

  it("copies the sibling code text when the copy button is clicked", async () => {
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, "clipboard", {
      value: { writeText },
      configurable: true,
    });

    const host = renderIntoDom("```\nlet x = 1;\n```");
    const btn = host.querySelector(".code-copy")!;
    const event = new MouseEvent("click", { bubbles: true });
    Object.defineProperty(event, "target", { value: btn });

    expect(handleCodeCopyClick(event)).toBe(true);
    expect(writeText).toHaveBeenCalledWith("let x = 1;\n");
    host.remove();
  });

  it("ignores clicks outside a copy button", () => {
    const host = renderIntoDom("plain paragraph");
    const p = host.querySelector("p")!;
    const event = new MouseEvent("click", { bubbles: true });
    Object.defineProperty(event, "target", { value: p });

    expect(handleCodeCopyClick(event)).toBe(false);
    host.remove();
  });
});
