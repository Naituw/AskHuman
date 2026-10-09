import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../i18n";
import { markdownReady } from "../lib/markdown";
import { applyTheme } from "../lib/theme";
import { openPath, localImageCreateScope, localImagePrepare, localImageReleaseScope } from "../lib/ipc";
import MarkdownContent from "./MarkdownContent.vue";

const renderMermaid = vi.hoisted(() => vi.fn());

vi.mock("../lib/mermaid", async (importOriginal) => {
  const original = await importOriginal<typeof import("../lib/mermaid")>();
  return { ...original, renderMermaid };
});

vi.mock("../lib/ipc", () => ({
  openPath: vi.fn(async () => {}),
  localImageCreateScope: vi.fn(async () => "scope"),
  localImagePrepare: vi.fn(async () => ({ token: "image", width: 320, height: 180 })),
  localImageReleaseScope: vi.fn(async () => {}),
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: (token: string, scheme: string) => `${scheme}://localhost/${token}`,
}));

const renderedDocument = {
  documentUrl: "data:text/html;charset=UTF-8;base64,PGh0bWw+PC9odG1sPg==",
  width: 320,
  height: 180,
  findText: "Alpha\nBeta",
  accTitle: "Flow",
  accDescription: "",
};

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

enableAutoUnmount(afterEach);

async function waitForRenderCount(count: number): Promise<void> {
  await vi.waitFor(() => expect(renderMermaid).toHaveBeenCalledTimes(count));
  await flushPromises();
}

describe("MarkdownContent", () => {
  beforeAll(() => markdownReady);

  beforeEach(() => {
    vi.mocked(openPath).mockClear();
    vi.mocked(localImageCreateScope).mockReset().mockResolvedValue("scope");
    vi.mocked(localImagePrepare).mockReset().mockResolvedValue({ token: "image", width: 320, height: 180 });
    vi.mocked(localImageReleaseScope).mockClear();
    vi.unstubAllGlobals();
    renderMermaid.mockReset();
    renderMermaid.mockResolvedValue(renderedDocument);
    i18n.global.locale.value = "en";
    applyTheme("light");
  });

  it.each(["tauri://localhost/index.html?view=popup-inbox", "http://tauri.localhost/index.html?view=popup-inbox"])("opens local file citations without navigating %s", async baseUrl => {
    const base = document.createElement("base");
    base.href = baseUrl; document.head.appendChild(base);
    try {
      const wrapper = mount(MarkdownContent, {
        props: { source: "[file](/Users/test/My%20Report.md:82:5) [relative](docs/report.md#L12)", baseDirectory: "/project" },
        global: { plugins: [i18n] }, attachTo: document.body,
      });
      await flushPromises();
      for (const anchor of wrapper.findAll("a")) {
        const click = new MouseEvent("click", { bubbles: true, cancelable: true });
        anchor.element.dispatchEvent(click);
        expect(click.defaultPrevented).toBe(true);
      }
      expect(openPath).toHaveBeenNthCalledWith(1, "/Users/test/My Report.md");
      expect(openPath).toHaveBeenNthCalledWith(2, "/project/docs/report.md");
    } finally { base.remove(); }
  });

  it("keeps unsupported links inert and opens supported URLs externally", async () => {
    const wrapper = mount(MarkdownContent, {
      props: { source: "[unsupported](ftp://example.com/file) [site](https://example.com)" },
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const anchors = wrapper.findAll("a");
    const unsupported = new MouseEvent("click", { bubbles: true, cancelable: true });
    anchors[0].element.dispatchEvent(unsupported);
    expect(unsupported.defaultPrevented).toBe(true);
    expect(openPath).not.toHaveBeenCalled();
    await anchors[1].trigger("click");
    expect(openPath).toHaveBeenCalledWith("https://example.com/");
  });

  it("keeps the document intact when the system opener rejects a file", async () => {
    vi.mocked(openPath).mockRejectedValueOnce(new Error("file unavailable"));
    const wrapper = mount(MarkdownContent, { props: { source: "[file](/missing/file.md)" }, global: { plugins: [i18n] } });
    await flushPromises();
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    wrapper.get("a").element.dispatchEvent(click);
    await flushPromises();
    expect(click.defaultPrevented).toBe(true);
    expect(wrapper.get("a").text()).toBe("file");
  });

  it.each(["tauri://localhost/index.html?view=popup-inbox", "http://tauri.localhost/index.html?view=popup-inbox"])("registers authored local images instead of fetching them from %s", async baseUrl => {
    const base = document.createElement("base"); base.href = baseUrl; document.head.appendChild(base);
    try {
      const wrapper = mount(MarkdownContent, {
        props: { source: "![preview](/Users/test/My%20Preview.png) ![relative](images/chart.png) ![file](file:///Users/test/图.png)", baseDirectory: "/project" },
        global: { plugins: [i18n] },
      });
      await flushPromises();
      expect(localImagePrepare).toHaveBeenNthCalledWith(1, "scope", "/Users/test/My Preview.png");
      expect(localImagePrepare).toHaveBeenNthCalledWith(2, "scope", "/project/images/chart.png");
      expect(localImagePrepare).toHaveBeenNthCalledWith(3, "scope", "/Users/test/图.png");
      expect(wrapper.findAll("img").map(image => image.attributes("src"))).toEqual(Array(3).fill("askhuman-image://localhost/image"));
    } finally { base.remove(); }
  });

  it("shares a read for repeated images and leaves remote images unchanged", async () => {
    const wrapper = mount(MarkdownContent, {
      props: { source: "![one](/tmp/image.png) ![two](/tmp/image.png) ![remote](https://example.com/image.png)" },
      global: { plugins: [i18n] },
    });
    await flushPromises();
    expect(localImagePrepare).toHaveBeenCalledTimes(1);
    expect(wrapper.findAll("img")[2].attributes("src")).toBe("https://example.com/image.png");
  });

  it("shows the image label when the file is missing or cannot be decoded", async () => {
    vi.mocked(localImagePrepare).mockRejectedValueOnce(new Error("missing file"));
    const wrapper = mount(MarkdownContent, {
      props: { source: "![missing](/tmp/missing.png) ![invalid](/tmp/invalid.png) ![no project](image.png)" },
      global: { plugins: [i18n] },
    });
    await flushPromises();
    await wrapper.get("img").trigger("error");
    expect(wrapper.findAll("img")).toHaveLength(0);
    expect(wrapper.text()).toContain("Image could not be displayed · missing");
    expect(wrapper.text()).toContain("Image could not be displayed · invalid");
    expect(wrapper.text()).toContain("Image could not be displayed · no project");
  });

  it("discards stale image reads after a project or source change", async () => {
    const oldImage = deferred<{ token: string; width: number; height: number }>();
    vi.mocked(localImagePrepare).mockImplementationOnce(() => oldImage.promise);
    const wrapper = mount(MarkdownContent, {
      props: { source: "![preview](image.png)", baseDirectory: "/old" },
      global: { plugins: [i18n] },
    });
    await flushPromises();
    const removed = wrapper.get("img").element;
    expect(removed.hasAttribute("src")).toBe(false);
    await wrapper.setProps({ baseDirectory: "/new" }); await flushPromises();
    oldImage.resolve({ token: "old", width: 320, height: 180 }); await flushPromises();
    expect(removed.hasAttribute("src")).toBe(false);
    expect(localImagePrepare).toHaveBeenLastCalledWith("scope", "/new/image.png");
    expect(wrapper.get("img").attributes("src")).toBe("askhuman-image://localhost/image");
  });

  it("defers disk work until a local image approaches the viewport", async () => {
    let notify!: IntersectionObserverCallback;
    const observed = new Set<Element>();
    const disconnect = vi.fn(() => observed.clear());
    vi.stubGlobal("IntersectionObserver", class {
      constructor(callback: IntersectionObserverCallback) { notify = callback; }
      observe(target: Element) { observed.add(target); }
      unobserve(target: Element) { observed.delete(target); }
      disconnect = disconnect;
    });
    const wrapper = mount(MarkdownContent, { props: { source: "![preview](/tmp/image.png)", enableMermaid: false }, global: { plugins: [i18n] } });
    await flushPromises();
    const image = wrapper.get("img").element;
    expect(observed.has(image)).toBe(true);
    expect(localImageCreateScope).not.toHaveBeenCalled();
    notify([{ target: image, isIntersecting: true, intersectionRatio: 1, boundingClientRect: image.getBoundingClientRect(), intersectionRect: image.getBoundingClientRect(), rootBounds: null, time: 0 }], {} as IntersectionObserver);
    await flushPromises();
    expect(localImagePrepare).toHaveBeenCalledWith("scope", "/tmp/image.png");
    expect(image.getAttribute("src")).toBe("askhuman-image://localhost/image");
    expect(image.getAttribute("loading")).toBe("lazy");
    wrapper.unmount(); await flushPromises();
    expect(disconnect).toHaveBeenCalled();
    expect(localImageReleaseScope).toHaveBeenCalledWith("scope");
    expect(image.hasAttribute("src")).toBe(false);
  });

  it("releases a scope that arrives after the document unmounts", async () => {
    const pending = deferred<string>(); vi.mocked(localImageCreateScope).mockReturnValueOnce(pending.promise);
    const wrapper = mount(MarkdownContent, { props: { source: "![preview](/tmp/image.png)" }, global: { plugins: [i18n] } });
    await flushPromises(); wrapper.unmount(); pending.resolve("late"); await flushPromises();
    expect(localImageReleaseScope).toHaveBeenCalledWith("late");
    expect(localImagePrepare).not.toHaveBeenCalled();
  });

  it("opens original images by click or keyboard, including over-limit fallbacks", async () => {
    vi.mocked(localImagePrepare).mockRejectedValueOnce("limit");
    const wrapper = mount(MarkdownContent, { props: { source: "![large](/tmp/large.png) ![small](/tmp/small.png)" }, global: { plugins: [i18n] } });
    await flushPromises();
    expect(wrapper.get(".markdown-image-error").text()).toContain("Image is too large");
    await wrapper.get(".markdown-image-error").trigger("click");
    expect(openPath).toHaveBeenLastCalledWith("/tmp/large.png");
    await wrapper.get("img").trigger("keydown", { key: "Enter" });
    expect(openPath).toHaveBeenLastCalledWith("/tmp/small.png");
    await wrapper.get("img").trigger("click");
    expect(openPath).toHaveBeenCalledTimes(3);
  });

  it("progressively replaces a Mermaid fence and preserves its source", async () => {
    const wrapper = mount(MarkdownContent, {
      props: { source: "```mermaid\nflowchart TD\nA-->B\n```" },
      attrs: { style: "font-size: 13px" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);

    expect(renderMermaid).toHaveBeenCalledWith(
      "flowchart TD\nA-->B\n",
      "light",
      13,
    );
    expect(wrapper.find(".mermaid-frame").attributes("sandbox")).toBe("");
    expect(wrapper.find(".mermaid-frame-shell").exists()).toBe(true);
    expect(wrapper.find(".mermaid-frame").attributes("title")).toBe("Flow");
    expect((wrapper.find("pre").element as HTMLElement).hidden).toBe(true);
    expect(wrapper.find("code").text()).toBe("flowchart TD\nA-->B");
    expect(wrapper.find("[data-find-atomic]").exists()).toBe(true);

    const canvas = wrapper.get(".mermaid-canvas").element as HTMLElement;
    Object.defineProperty(canvas, "clientWidth", {
      configurable: true,
      value: 300,
    });
    window.dispatchEvent(new Event("resize"));
    await vi.waitFor(() =>
      expect(wrapper.get(".mermaid-frame-shell").attributes("style")).toContain(
        "width: 300px",
      ),
    );
    expect(wrapper.get(".mermaid-frame").attributes("style")).toContain(
      "scale(0.9375)",
    );
  });

  it("switches between rendered and source find modes", async () => {
    const wrapper = mount(MarkdownContent, {
      props: { source: "```mermaid\nflowchart TD\nA-->B\n```" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);

    await wrapper.find(".mermaid-source-toggle").trigger("click");
    expect((wrapper.find("pre").element as HTMLElement).hidden).toBe(false);
    expect(
      (wrapper.find(".mermaid-canvas").element as HTMLElement).hidden,
    ).toBe(true);
    expect(wrapper.find("[data-find-atomic]").exists()).toBe(false);
    expect(wrapper.find("pre").attributes("data-find-skip")).toBeUndefined();
  });

  it("keeps source visible on render failure", async () => {
    renderMermaid.mockRejectedValueOnce(new Error("bad syntax"));
    const wrapper = mount(MarkdownContent, {
      props: { source: "```mermaid\nnot a diagram\n```" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);

    expect((wrapper.find("pre").element as HTMLElement).hidden).toBe(false);
    expect(wrapper.find(".mermaid-status-failed").text()).toBe(
      "Diagram could not be rendered; showing source",
    );
  });

  it("limits each Markdown body to ten diagrams", async () => {
    const source = Array.from(
      { length: 11 },
      (_, index) => `\`\`\`mermaid\nflowchart TD\nA${index}-->B${index}\n\`\`\``,
    ).join("\n");
    const wrapper = mount(MarkdownContent, {
      props: { source },
      global: { plugins: [i18n] },
    });
    expect(wrapper.findAll(".mermaid-block")).toHaveLength(11);
    await waitForRenderCount(10);

    expect(renderMermaid).toHaveBeenCalledTimes(10);
    expect(wrapper.findAll(".mermaid-status-tooMany")).toHaveLength(1);
  });

  it("redraws from source when the effective theme changes", async () => {
    mount(MarkdownContent, {
      props: { source: "```mermaid\nflowchart TD\nA-->B\n```" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);
    applyTheme("dark");
    await waitForRenderCount(2);

    expect(renderMermaid).toHaveBeenLastCalledWith(
      "flowchart TD\nA-->B\n",
      "dark",
      14,
    );
    expect(renderMermaid).toHaveBeenCalledTimes(2);
  });

  it("rejects oversized source before loading the adapter", async () => {
    const source = `\`\`\`mermaid\n${"x".repeat(40_001)}\n\`\`\``;
    const wrapper = mount(MarkdownContent, {
      props: { source },
      global: { plugins: [i18n] },
    });
    await vi.waitFor(() =>
      expect(wrapper.find(".mermaid-status-tooLarge").exists()).toBe(true),
    );
    expect(renderMermaid).not.toHaveBeenCalled();
    expect((wrapper.find("pre").element as HTMLElement).hidden).toBe(false);
  });

  it("does not write a stale diagram after the source changes", async () => {
    const first = deferred<typeof renderedDocument>();
    const second = deferred<typeof renderedDocument>();
    renderMermaid
      .mockImplementationOnce(() => first.promise)
      .mockImplementationOnce(() => second.promise);
    const wrapper = mount(MarkdownContent, {
      props: { source: "```mermaid\nflowchart TD\nOld-->Value\n```" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);
    await wrapper.setProps({
      source: "```mermaid\nflowchart TD\nNew-->Value\n```",
    });
    await waitForRenderCount(2);

    first.resolve({ ...renderedDocument, accTitle: "Old" });
    await flushPromises();
    expect(wrapper.find(".mermaid-frame").exists()).toBe(false);

    second.resolve({ ...renderedDocument, accTitle: "New" });
    await vi.waitFor(() =>
      expect(wrapper.find(".mermaid-frame").attributes("title")).toBe("New"),
    );
    expect(wrapper.find("code").text()).toContain("New-->Value");
  });

  it("redraws localized controls when the locale changes", async () => {
    const wrapper = mount(MarkdownContent, {
      props: { source: "```mermaid\nflowchart TD\nA-->B\n```" },
      global: { plugins: [i18n] },
    });
    await waitForRenderCount(1);
    i18n.global.locale.value = "zh";
    await waitForRenderCount(2);

    expect(wrapper.find(".mermaid-source-toggle").text()).toBe(
      "查看 Mermaid 源码",
    );
  });
});
