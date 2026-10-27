import { mount } from "@vue/test-utils";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { TodoProjectInfo, TodoSyncProject } from "../../lib/types";
import TodosSidebar from "./TodosSidebar.vue";
import { ALL_KEY, buildSidebarModel } from "./todoProjects";

const projects: TodoProjectInfo[] = [
  { key: "/w/a/app", name: "app", count: 4, section: "withTodos" },
  { key: "/w/b/app", name: "app", count: 2, section: "withTodos" },
  { key: "/w/blog", name: "blog", count: 0, section: "recent" },
];

function mountSidebar(props: { selectedKey?: string; query?: string; slots?: Record<string, string> } = {}) {
  const query = props.query ?? "";
  return mount(TodosSidebar, {
    attachTo: document.body,
    props: {
      model: buildSidebarModel(projects, query),
      selectedKey: props.selectedKey ?? "/w/a/app",
      query,
    },
    slots: props.slots,
    global: { plugins: [i18n] },
  });
}

describe("TodosSidebar", () => {
  beforeEach(() => {
    i18n.global.locale.value = "zh";
    // jsdom has no layout, hence no scrollIntoView.
    Element.prototype.scrollIntoView = vi.fn();
  });
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders the aggregate row, the sections, counts and disambiguation hints", () => {
    const wrapper = mountSidebar();
    const options = wrapper.findAll("[role=option]");
    expect(options.map((o) => o.attributes("data-key"))).toEqual([
      ALL_KEY,
      "/w/a/app",
      "/w/b/app",
      "/w/blog",
    ]);
    expect(options[0].text()).toContain("全部待办");
    expect(options[0].text()).toContain("6");
    expect(options[1].text()).toContain("app · a");
    expect(options[1].text()).toContain("4");
    expect(options[2].text()).toContain("app · b");
    // No badge for a project without todos.
    expect(options[3].find(".td-side-count").exists()).toBe(false);
    expect(wrapper.findAll(".td-side-section-title").map((t) => t.text())).toEqual([
      "有待办",
      "最近项目",
    ]);
  });

  it("marks the selected row and keeps a single tab stop on it", () => {
    const wrapper = mountSidebar({ selectedKey: "/w/b/app" });
    const selected = wrapper.findAll("[role=option]").filter((o) => o.attributes("aria-selected") === "true");
    expect(selected).toHaveLength(1);
    expect(selected[0].attributes("data-key")).toBe("/w/b/app");
    const stops = wrapper.findAll("[role=option]").filter((o) => o.attributes("tabindex") === "0");
    expect(stops.map((o) => o.attributes("data-key"))).toEqual(["/w/b/app"]);
  });

  it("falls back to the first row as the tab stop when the selection is filtered out", () => {
    const wrapper = mountSidebar({ selectedKey: "/w/a/app", query: "blog" });
    const stops = wrapper.findAll("[role=option]").filter((o) => o.attributes("tabindex") === "0");
    expect(stops.map((o) => o.attributes("data-key"))).toEqual([ALL_KEY]);
  });

  it("emits select when a row is clicked", async () => {
    const wrapper = mountSidebar();
    await wrapper.get('[data-key="/w/blog"]').trigger("click");
    await wrapper.get(`[data-key="${ALL_KEY}"]`).trigger("click");
    expect(wrapper.emitted("select")).toEqual([["/w/blog"], [ALL_KEY]]);
  });

  it("steps the selection with the arrow keys, clamps at the ends and keeps the keys to itself", async () => {
    const wrapper = mountSidebar({ selectedKey: "/w/a/app" });
    const list = wrapper.get("[role=listbox]");
    const reached: boolean[] = [];
    document.body.addEventListener("keydown", () => reached.push(true));

    await wrapper.get('[data-key="/w/a/app"]').trigger("keydown", { key: "ArrowDown" });
    expect(wrapper.emitted("select")?.[0]).toEqual(["/w/b/app"]);
    await wrapper.get('[data-key="/w/a/app"]').trigger("keydown", { key: "ArrowUp" });
    expect(wrapper.emitted("select")?.[1]).toEqual([ALL_KEY]);
    await list.trigger("keydown", { key: "End" });
    expect(wrapper.emitted("select")?.[2]).toEqual(["/w/blog"]);
    await list.trigger("keydown", { key: "Home" });
    expect(wrapper.emitted("select")?.[3]).toEqual([ALL_KEY]);
    // The window-level todo navigation must not see the arrows.
    expect(reached).toEqual([]);
  });

  it("does not re-emit when the arrow key lands on the already selected row", async () => {
    const wrapper = mountSidebar({ selectedKey: "/w/blog" });
    await wrapper.get('[data-key="/w/blog"]').trigger("keydown", { key: "ArrowDown" });
    expect(wrapper.emitted("select")).toBeUndefined();
  });

  it("moves focus along with the arrow keys", async () => {
    const wrapper = mountSidebar({ selectedKey: "/w/a/app" });
    await wrapper.get('[data-key="/w/a/app"]').trigger("keydown", { key: "ArrowDown" });
    await new Promise((resolve) => setTimeout(resolve));
    expect((document.activeElement as HTMLElement | null)?.dataset.key).toBe("/w/b/app");
  });

  it("reports typing in the search box and clears it with Escape", async () => {
    const wrapper = mountSidebar({ query: "bl" });
    const input = wrapper.get<HTMLInputElement>(".td-side-input");
    await input.setValue("blo");
    expect(wrapper.emitted("update:query")?.[0]).toEqual(["blo"]);
    await input.trigger("keydown", { key: "Escape" });
    expect(wrapper.emitted("update:query")?.[1]).toEqual([""]);
  });

  it("selects the first match on Enter and moves into the list on ArrowDown", async () => {
    const wrapper = mountSidebar({ query: "app" });
    const input = wrapper.get(".td-side-input");
    await input.trigger("keydown", { key: "Enter" });
    expect(wrapper.emitted("select")?.[0]).toEqual(["/w/a/app"]);
    await input.trigger("keydown", { key: "ArrowDown" });
    await new Promise((resolve) => setTimeout(resolve));
    expect((document.activeElement as HTMLElement | null)?.dataset.key).toBe("/w/a/app");
  });

  it("ignores Enter while an IME composition is active", async () => {
    const wrapper = mountSidebar({ query: "app" });
    await wrapper.get(".td-side-input").trigger("keydown", { key: "Enter", isComposing: true });
    expect(wrapper.emitted("select")).toBeUndefined();
  });

  it("says so when nothing matches", () => {
    const wrapper = mountSidebar({ query: "zzz" });
    expect(wrapper.find(".td-side-empty").text()).toBe("没有匹配的项目");
  });

  it("renders the footer strip only when a footer is provided", () => {
    expect(mountSidebar().find(".td-side-foot").exists()).toBe(false);
    const wrapper = mountSidebar({ slots: { footer: "<span>已同步</span>" } });
    expect(wrapper.get(".td-side-foot").text()).toBe("已同步");
  });

  it("scrolls the selected row into view once, after the rows arrive", async () => {
    const scroll = vi.fn();
    Element.prototype.scrollIntoView = scroll;
    // Mounted before the data: nothing to reveal yet.
    const wrapper = mount(TodosSidebar, {
      attachTo: document.body,
      props: { model: buildSidebarModel([], ""), selectedKey: "/w/b/app", query: "" },
      global: { plugins: [i18n] },
    });
    await new Promise((resolve) => setTimeout(resolve));
    expect(scroll).not.toHaveBeenCalled();

    await wrapper.setProps({ model: buildSidebarModel(projects, "") });
    await new Promise((resolve) => setTimeout(resolve));
    expect(scroll).toHaveBeenCalledTimes(1);
    expect((scroll.mock.contexts[0] as HTMLElement).dataset.key).toBe("/w/b/app");

    // Later selection changes are the user's own doing and are not scrolled to again.
    await wrapper.setProps({ selectedKey: "/w/blog" });
    await new Promise((resolve) => setTimeout(resolve));
    expect(scroll).toHaveBeenCalledTimes(1);
  });

  it("exposes focusSearch", () => {
    const wrapper = mountSidebar();
    (wrapper.vm as unknown as { focusSearch: () => void }).focusSearch();
    expect(document.activeElement).toBe(wrapper.get(".td-side-input").element);
  });
});

describe("TodosSidebar in iCloud mode", () => {
  const cloudKey = "icloud:github.com/acme/dotfiles";
  const world: TodoProjectInfo[] = [
    { key: "/w/app-wt", name: "app-wt", count: 3, section: "withTodos" },
    { key: "/w/app", name: "app", count: 3, section: "withTodos" },
    { key: cloudKey, name: "dotfiles", count: 2, section: "withTodos" },
    { key: "/w/notes", name: "notes", count: 1, section: "withTodos" },
    { key: "/w/blog", name: "blog", count: 0, section: "recent" },
  ];
  const sync: TodoSyncProject[] = [
    { identity: "github.com/acme/app", keys: ["/w/app", "/w/app-wt"], cloudKey: null, macs: [] },
    { identity: "github.com/acme/dotfiles", keys: [], cloudKey, macs: [] },
    { identity: "github.com/acme/blog", keys: ["/w/blog"], cloudKey: null, macs: [] },
  ];

  function mountSynced(opts: { emptyOpen?: boolean; query?: string; selectedKey?: string } = {}) {
    const query = opts.query ?? "";
    return mount(TodosSidebar, {
      attachTo: document.body,
      props: {
        model: buildSidebarModel(world, query, "/Users/me", { sync, emptyOpen: opts.emptyOpen }),
        selectedKey: opts.selectedKey ?? "/w/app-wt",
        query,
      },
      global: { plugins: [i18n] },
    });
  }

  beforeEach(() => {
    i18n.global.locale.value = "zh";
    Element.prototype.scrollIntoView = vi.fn();
  });
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("titles the sections and folds the projects without todos into one row", () => {
    const wrapper = mountSynced();
    expect(wrapper.findAll(".td-side-section-title").map((t) => t.text())).toEqual([
      "iCloud 同步的项目",
      "仅本机（不同步）",
    ]);
    const fold = wrapper.get(".td-side-fold");
    expect(fold.text()).toContain("无待办的项目（1）");
    expect(fold.attributes("aria-expanded")).toBe("false");
    // The fold row is a button but not an option: it takes no part in selection or arrow keys.
    expect(fold.attributes("role")).toBeUndefined();
    expect(wrapper.findAll("[role=option]").map((o) => o.attributes("data-key"))).toEqual([
      ALL_KEY,
      "/w/app-wt",
      cloudKey,
      "/w/notes",
    ]);
  });

  it("marks projects that only exist in iCloud, or only here", () => {
    const wrapper = mountSynced();
    const row = (key: string) => wrapper.get(`[data-key="${key}"]`);
    expect(row(cloudKey).find(".td-side-scope").attributes("aria-label")).toBe("仅云端");
    expect(row("/w/notes").find(".td-side-scope").attributes("aria-label")).toBe("仅本机");
    expect(row("/w/app-wt").find(".td-side-scope").exists()).toBe(false);
    // One row for the two directories of the repository, named by its address.
    expect(row("/w/app-wt").text()).toContain("app");
    expect(wrapper.find('[data-key="/w/app"]').exists()).toBe(false);
  });

  it("asks the parent to toggle the fold and lists the rows once it is open", async () => {
    const wrapper = mountSynced();
    await wrapper.get(".td-side-fold").trigger("click");
    expect(wrapper.emitted("toggle-empty")).toHaveLength(1);

    const open = mountSynced({ emptyOpen: true });
    expect(open.get(".td-side-fold").attributes("aria-expanded")).toBe("true");
    expect(open.get('[data-key="/w/blog"]').classes()).toContain("nested");
    expect(open.findAll("[role=option]").map((o) => o.attributes("data-key"))).toContain("/w/blog");
  });

  it("highlights the row that merges the selected directory", () => {
    const wrapper = mountSynced({ selectedKey: "/w/app-wt" });
    const selected = wrapper.findAll("[role=option]").filter((o) => o.attributes("aria-selected") === "true");
    expect(selected.map((o) => o.attributes("data-key"))).toEqual(["/w/app-wt"]);
  });
});

