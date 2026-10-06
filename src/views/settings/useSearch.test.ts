import { mount } from "@vue/test-utils";
import { defineComponent, h, nextTick, ref } from "vue";
import { describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import type { AppConfig } from "../../lib/types";
import type { Tab } from "./context";
import { useSettingsSearch } from "./useSearch";

describe("useSettingsSearch", () => {
  it.each([
    ["zh", "提问窗口模式", ["合并", "独立", "窗口模式", "Sidebar", "弹窗模式", "作答窗口"]],
    ["en", "Question window mode", ["merged", "independent", "window mode", "Sidebar"]],
  ] as const)("finds window modes and navigates to the visible General row in %s", async (locale, title, queries) => {
    i18n.global.locale.value = locale;
    let search!: ReturnType<typeof useSettingsSearch>;
    const activeTab = ref<Tab>("channel");
    const wrapper = mount(defineComponent({
      setup() {
        search = useSettingsSearch({ config: ref<AppConfig | null>(null), activeTab });
        return () => h("div", { class: "settings-body" }, activeTab.value === "general"
          ? h("div", { class: "row" }, h("span", { class: "label" }, title)) : []);
      },
    }), { attachTo: document.body, global: { plugins: [i18n] } });
    const scroll = vi.fn();
    const previous = HTMLElement.prototype.scrollIntoView;
    HTMLElement.prototype.scrollIntoView = scroll;
    vi.useFakeTimers();
    try {
      for (const query of queries) {
        search.searchQuery.value = query;
        await nextTick();
        expect(search.searchResults.value).toContainEqual({ tab: "general", title, extra: expect.any(Array) });
      }
      await search.gotoSearchResult(search.searchResults.value.find(entry => entry.title === title)!);
      expect(activeTab.value).toBe("general");
      expect(wrapper.find(".row").classes()).toContain("search-hit-highlight");
      expect(scroll).toHaveBeenCalledWith({ behavior: "smooth", block: "center" });
      vi.runAllTimers();
    } finally {
      wrapper.unmount();
      HTMLElement.prototype.scrollIntoView = previous;
      vi.useRealTimers();
      i18n.global.locale.value = "en";
    }
  });
  it("indexes the Pi integration card", async () => {
    i18n.global.locale.value = "en";
    let search!: ReturnType<typeof useSettingsSearch>;
    const harness = defineComponent({
      setup() {
        search = useSettingsSearch({
          config: ref<AppConfig | null>(null),
          activeTab: ref<Tab>("general"),
        });
        return () => null;
      },
    });
    const wrapper = mount(harness, { global: { plugins: [i18n] } });

    search.searchQuery.value = "pi";
    await nextTick();

    expect(search.searchResults.value).toContainEqual({
      tab: "integration",
      title: "Pi",
      extra: ["Agent"],
    });

    search.searchQuery.value = "lifecycle tracking";
    await nextTick();
    expect(search.searchResults.value).toContainEqual({
      tab: "integration",
      title: "Lifecycle tracking",
      extra: expect.any(Array),
    });
    wrapper.unmount();
  });
});
