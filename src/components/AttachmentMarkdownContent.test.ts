import { enableAutoUnmount, flushPromises, mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../i18n";
import { applyTheme } from "../lib/theme";
import AttachmentMarkdownContent from "./AttachmentMarkdownContent.vue";

const render = vi.hoisted(() => vi.fn());
vi.mock("../lib/mermaidLoader", () => ({ loadMermaidAdapter: async () => ({ renderMermaid: render }) }));
const html = '<p>Body</p><a href="">Blocked</a><div class="mermaid-block" data-mermaid-pending><pre><code class="language-mermaid">flowchart TD\nA--&gt;B\n</code></pre></div>';
enableAutoUnmount(afterEach);

describe("attachment Markdown content", () => {
  beforeEach(() => {
    applyTheme("light");
    i18n.global.locale.value = "en";
    render.mockReset().mockResolvedValue({ documentUrl: "data:text/html;charset=UTF-8;base64,PGh0bWw+PC9odG1sPg==", width: 300, height: 180, findText: "A B", accTitle: "Flow", accDescription: "" });
  });
  it("renders backend-marked diagrams and redraws for appearance without changing the URL policy", async () => {
    const wrapper = mount(AttachmentMarkdownContent, { props: { html }, global: { plugins: [i18n] }, attachTo: document.body });
    await vi.waitFor(() => expect(wrapper.find("iframe").exists()).toBe(true));
    expect(render).toHaveBeenLastCalledWith("flowchart TD\nA-->B\n", "light", 14);
    expect(wrapper.get("a").attributes("href")).toBe("");
    applyTheme("dark");
    await vi.waitFor(() => expect(render).toHaveBeenCalledTimes(2));
    await flushPromises();
    expect(render).toHaveBeenLastCalledWith("flowchart TD\nA-->B\n", "dark", 14);
    expect(wrapper.findAll("iframe")).toHaveLength(1);
    await wrapper.setProps({ html: '<pre><code class="language-less">flowchart TD</code></pre>' });
    await flushPromises();
    expect(wrapper.findAll("iframe")).toHaveLength(0);
    expect(render).toHaveBeenCalledTimes(2);
  });
});
