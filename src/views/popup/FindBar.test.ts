import { defineComponent, h, nextTick, provide, ref } from "vue";
import { enableAutoUnmount, mount } from "@vue/test-utils";
import { afterEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import { PopupCtxKey, type PopupContext } from "./context";
import FindBar from "./FindBar.vue";
import type { FindScope } from "./attachmentFind";
enableAutoUnmount(afterEach);

describe("find bar moving between sibling panes", () => {
  it("keeps the shared input ref and focus when the new main bar mounts before the old attachment bar unmounts", async () => {
    const scope = ref<FindScope>("attachment"), input = ref<HTMLInputElement | null>(null);
    const ctx = {
      findScope: scope, findStatus: ref("ready"), previewOpen: ref(true), previewFile: ref(null),
      openFind: (_prefill: boolean, target: FindScope) => { scope.value = target; }, openFile: vi.fn(),
      findTotal: ref(2), findQuery: ref("alpha"), findCaseSensitive: ref(false), findCountLabel: ref("1/2"),
      findNoMatch: ref(false), findInputEl: input, closeFind: vi.fn(), goFind: vi.fn(), onFindQueryInput: vi.fn(), toggleFindCase: vi.fn(),
    } as unknown as PopupContext;
    const wrapper = mount(defineComponent({ setup() {
      provide(PopupCtxKey, ctx);
      return () => h("div", [h("main", scope.value === "question" ? [h(FindBar)] : []),
        h("aside", scope.value === "attachment" ? [h(FindBar)] : [])]);
    } }), { attachTo: document.body, global: { plugins: [i18n] } });
    await nextTick(); scope.value = "question"; await nextTick(); await nextTick();
    expect(wrapper.findAll(".popup-find-bar")).toHaveLength(1);
    expect(input.value).toBe(wrapper.get("main input").element);
    expect(document.activeElement).toBe(input.value);
    scope.value = "attachment"; await nextTick(); await nextTick();
    expect(input.value).toBe(wrapper.get("aside input").element);
    expect(document.activeElement).toBe(input.value);
  });
});
