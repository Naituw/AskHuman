import { afterEach, describe, expect, it, vi } from "vitest";
import { enhanceAttachmentMermaid, type AttachmentMermaidLabels } from "./attachmentMermaid";

const labels: AttachmentMermaidLabels = {
  copy: "Copy", copied: "Copied", diagram: "Diagram", showSource: "Source", showDiagram: "Diagram",
  rendering: "Rendering", failed: "Failed", tooLarge: "Too large", tooMany: "Too many",
};
const rendered = { documentUrl: "data:text/html;charset=UTF-8;base64,PGh0bWw+PC9odG1sPg==", width: 600, height: 200, findText: "Scene", accTitle: "Scene", accDescription: "" };
const cleanups: Array<() => void> = [];
function host(count = 1) {
  const root = document.createElement("article");
  root.style.fontSize = "14px";
  root.innerHTML = '<p>Keep paragraph</p><a href="">Blocked link</a>' + '<div class="mermaid-block" data-mermaid-pending><pre><code class="language-mermaid">flowchart TD\nA--&gt;B\n</code></pre></div>'.repeat(count);
  document.body.appendChild(root);
  cleanups.push(() => root.remove());
  return root;
}
afterEach(() => { for (const cleanup of cleanups.splice(0)) cleanup(); });

describe("attachment Mermaid enhancement", () => {
  it("keeps backend HTML and code intact, isolates diagrams, fits width and toggles source", async () => {
    const root = host();
    Object.defineProperty(root, "clientWidth", { value: 480 });
    const render = vi.fn(async () => rendered);
    const controller = enhanceAttachmentMermaid(root, { labels, theme: "dark", render });
    cleanups.push(controller.dispose);
    await controller.done;
    expect(render).toHaveBeenCalledWith("flowchart TD\nA-->B\n", "dark", 14);
    expect(root.querySelector("a")?.getAttribute("href")).toBe("");
    expect(root.querySelector("p")?.textContent).toBe("Keep paragraph");
    expect(root.querySelector("iframe")?.getAttribute("sandbox")).toBe("");
    expect(root.querySelector("iframe")?.title).toBe("Scene");
    expect(root.querySelector<HTMLElement>(".am-shell")?.style.width).toBe("515px");
    const pre = root.querySelector("pre")!;
    expect(pre.hidden).toBe(true);
    expect(root.querySelector("[data-find-atomic]")).not.toBeNull();
    const toggle = root.querySelectorAll("button")[1]!;
    toggle.click();
    expect(pre.hidden).toBe(false);
    expect(root.querySelector("[data-find-atomic]")).toBeNull();
    expect(pre.textContent).toBe("flowchart TD\nA-->B\n");
    toggle.click();
    expect(pre.hidden).toBe(true);
  });

  it("retains failed source and limits diagrams before rendering", async () => {
    const root = host(11);
    const render = vi.fn(async () => { throw new Error("invalid graph"); });
    const controller = enhanceAttachmentMermaid(root, { labels, theme: "light", render });
    cleanups.push(controller.dispose);
    await controller.done;
    expect(render).toHaveBeenCalledTimes(10);
    expect(root.querySelectorAll("iframe")).toHaveLength(0);
    expect(Array.from(root.querySelectorAll("pre")).every(pre => !pre.hidden)).toBe(true);
    expect(root.textContent).toContain("Too many");
    expect(root.textContent).toContain("Failed");
  });

  it("does not load the renderer for ordinary code or oversized source", async () => {
    const root = host();
    root.querySelector("code")!.textContent = "x".repeat(40_001);
    const render = vi.fn(async () => rendered);
    const controller = enhanceAttachmentMermaid(root, { labels, theme: "light", render });
    cleanups.push(controller.dispose);
    await controller.done;
    expect(render).not.toHaveBeenCalled();
    expect(root.textContent).toContain("Too large");
    root.innerHTML = '<pre><code class="language-less">flowchart TD</code></pre>';
    await enhanceAttachmentMermaid(root, { labels, theme: "light", render }).done;
    expect(render).not.toHaveBeenCalled();
  });

  it("drops results after disposal or replacement by another attachment", async () => {
    const root = host();
    let finish!: (value: typeof rendered) => void;
    const render = vi.fn(() => new Promise<typeof rendered>(resolve => { finish = resolve; }));
    const controller = enhanceAttachmentMermaid(root, { labels, theme: "light", render });
    controller.dispose();
    root.innerHTML = "Other attachment";
    finish(rendered);
    await controller.done;
    expect(root.textContent).toBe("Other attachment");
    expect(root.querySelector("iframe")).toBeNull();
  });
});
