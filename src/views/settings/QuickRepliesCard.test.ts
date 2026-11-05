import { flushPromises, mount } from "@vue/test-utils";
import { reactive, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import QuickRepliesCard from "./QuickRepliesCard.vue";
import { QUICK_REPLIES_MAX, QUICK_REPLY_MAX_CHARS, sanitizeQuickReplies } from "./quickReplies";

const useSettingsContext = vi.hoisted(() => vi.fn());
const quickReplyDefaults = vi.hoisted(() => vi.fn());

vi.mock("./context", () => ({ useSettingsContext }));
vi.mock("../../lib/ipc", () => ({ quickReplyDefaults }));

const FACTORY = {
  en: ["Continue", "Use your own judgement", "Skip this one"],
  zh: ["继续", "按你的判断来", "先跳过这个"],
};

describe("QuickRepliesCard", () => {
  const persist = vi.fn(async () => {});

  async function setup(quickReplies: string[] | null) {
    const config = ref({ general: reactive({ quickReplies }) });
    useSettingsContext.mockReturnValue({ config, persist, settingsTargetHighlight: ref("") });
    const wrapper = mount(QuickRepliesCard, { global: { plugins: [i18n] }, attachTo: document.body });
    await flushPromises();
    return { config, wrapper };
  }

  // jsdom has no PointerEvent, and the test utils cannot fill a MouseEvent's `button`: dispatch plain mouse events.
  const pointer = (target: Element, type: string, clientY = 0) => {
    const event = new MouseEvent(type, { bubbles: true, cancelable: true, button: 0, clientY });
    Object.defineProperty(event, "pointerId", { value: 1 });
    target.dispatchEvent(event);
  };

  const inputs = (wrapper: ReturnType<typeof mount>) =>
    wrapper.findAll<HTMLInputElement>('[data-testid="quick-reply-input"]');
  const texts = (wrapper: ReturnType<typeof mount>) => inputs(wrapper).map((input) => input.element.value);

  beforeEach(() => {
    i18n.global.locale.value = "en";
    persist.mockClear();
    quickReplyDefaults.mockReset();
    quickReplyDefaults.mockResolvedValue(FACTORY);
    document.body.innerHTML = "";
  });

  it("shows the factory replies in the interface language until the list is edited", async () => {
    const { wrapper } = await setup(null);
    expect(texts(wrapper)).toEqual(FACTORY.en);
    expect(wrapper.find('[data-testid="quick-reply-defaults-hint"]').exists()).toBe(true);
    expect(wrapper.find('[data-testid="quick-reply-restore"]').exists()).toBe(false);

    i18n.global.locale.value = "zh";
    await flushPromises();
    expect(texts(wrapper)).toEqual(FACTORY.zh);
  });

  it("makes the first edit of the factory list the user's own list", async () => {
    const { wrapper, config } = await setup(null);
    const first = inputs(wrapper)[0];
    await first.setValue("Go ahead");

    expect(config.value.general.quickReplies).toEqual(["Go ahead", "Use your own judgement", "Skip this one"]);
    expect(persist).toHaveBeenCalledTimes(1);
    expect(wrapper.find('[data-testid="quick-reply-defaults-hint"]').exists()).toBe(false);
    expect(wrapper.find('[data-testid="quick-reply-restore"]').exists()).toBe(true);
  });

  it("shows the user's own list as it is, even when it is empty", async () => {
    const { wrapper } = await setup(["Only one"]);
    expect(texts(wrapper)).toEqual(["Only one"]);
    const empty = await setup([]);
    expect(texts(empty.wrapper)).toEqual([]);
  });

  it("deletes a row with its cross and saves", async () => {
    const { wrapper, config } = await setup(["a", "b", "c"]);
    await wrapper.findAll('[data-testid="quick-reply-remove"]')[1].trigger("click");
    expect(texts(wrapper)).toEqual(["a", "c"]);
    expect(config.value.general.quickReplies).toEqual(["a", "c"]);
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it("deletes a row whose text was emptied", async () => {
    const { wrapper, config } = await setup(["a", "b"]);
    const second = inputs(wrapper)[1];
    await second.setValue("   ");
    expect(texts(wrapper)).toEqual(["a"]);
    expect(config.value.general.quickReplies).toEqual(["a"]);
  });

  it("adds a focused blank row that is not stored until it has text", async () => {
    const { wrapper, config } = await setup(["a"]);
    await wrapper.get('[data-testid="quick-reply-add"]').trigger("click");
    await flushPromises();
    expect(inputs(wrapper)).toHaveLength(2);
    expect(document.activeElement).toBe(inputs(wrapper)[1].element);
    expect(persist).not.toHaveBeenCalled();

    const added = inputs(wrapper)[1];
    await added.setValue("  Looks good  ");
    expect(config.value.general.quickReplies).toEqual(["a", "Looks good"]);
  });

  it("drops a fresh row that was left blank", async () => {
    const { wrapper } = await setup(["a"]);
    await wrapper.get('[data-testid="quick-reply-add"]').trigger("click");
    await flushPromises();
    await inputs(wrapper)[1].trigger("blur");
    expect(texts(wrapper)).toEqual(["a"]);
    expect(persist).not.toHaveBeenCalled();
  });

  it("holds at most the ten the menu has digits for", async () => {
    const ten = Array.from({ length: QUICK_REPLIES_MAX }, (_, i) => `reply ${i + 1}`);
    const { wrapper } = await setup(ten);
    expect(wrapper.get('[data-testid="quick-reply-add"]').attributes("disabled")).toBeDefined();
    expect(wrapper.text()).toContain("The menu holds at most 10 replies");
  });

  it("limits one reply to a line of a sensible length", async () => {
    const { wrapper } = await setup(["a"]);
    expect(inputs(wrapper)[0].attributes("maxlength")).toBe(String(QUICK_REPLY_MAX_CHARS));
  });

  it("restores the factory list by forgetting the user's one", async () => {
    const { wrapper, config } = await setup(["mine"]);
    await wrapper.get('[data-testid="quick-reply-restore"]').trigger("click");
    expect(config.value.general.quickReplies).toBeNull();
    expect(texts(wrapper)).toEqual(FACTORY.en);
    expect(persist).toHaveBeenCalledTimes(1);
  });

  it("moves a row with the arrow keys on its handle and keeps the focus on it", async () => {
    const { wrapper, config } = await setup(["a", "b", "c"]);
    const handle = () => wrapper.findAll<HTMLButtonElement>('[data-testid="quick-reply-handle"]');
    handle()[0].element.focus();
    await handle()[0].trigger("keydown", { key: "ArrowDown" });
    await flushPromises();
    expect(texts(wrapper)).toEqual(["b", "a", "c"]);
    expect(config.value.general.quickReplies).toEqual(["b", "a", "c"]);
    expect(document.activeElement).toBe(handle()[1].element);

    await handle()[1].trigger("keydown", { key: "ArrowDown" });
    await flushPromises();
    expect(texts(wrapper)).toEqual(["b", "c", "a"]);
    // The last row cannot go further down.
    await handle()[2].trigger("keydown", { key: "ArrowDown" });
    expect(texts(wrapper)).toEqual(["b", "c", "a"]);
    expect(persist).toHaveBeenCalledTimes(2);
  });

  it("reorders by dragging a handle over the other rows and saves on release", async () => {
    const { wrapper, config } = await setup(["a", "b", "c"]);
    const rows = wrapper.findAll<HTMLElement>(".qrc-row");
    // jsdom has no layout: give the rows 30px slots.
    rows.forEach((row, i) => {
      row.element.getBoundingClientRect = () =>
        ({ top: i * 30, height: 30, bottom: i * 30 + 30, left: 0, right: 100, width: 100 }) as DOMRect;
    });
    const handle = wrapper.findAll('[data-testid="quick-reply-handle"]')[0].element;
    pointer(handle, "pointerdown");
    pointer(handle, "pointermove", 80);
    await flushPromises();
    expect(texts(wrapper)).toEqual(["b", "c", "a"]);
    expect(persist).not.toHaveBeenCalled();
    pointer(handle, "pointerup");
    await flushPromises();
    expect(config.value.general.quickReplies).toEqual(["b", "c", "a"]);
    expect(persist).toHaveBeenCalledTimes(1);
    // Moving the pointer after the release does nothing.
    pointer(handle, "pointermove", 0);
    await flushPromises();
    expect(texts(wrapper)).toEqual(["b", "c", "a"]);
  });
});

describe("sanitizeQuickReplies", () => {
  it("trims, drops blanks, cuts long replies and keeps ten", () => {
    expect(sanitizeQuickReplies(["  a  ", "", "   ", "b"])).toEqual(["a", "b"]);
    expect(sanitizeQuickReplies(["x".repeat(QUICK_REPLY_MAX_CHARS + 20)])[0]).toHaveLength(QUICK_REPLY_MAX_CHARS);
    expect(sanitizeQuickReplies(Array.from({ length: 14 }, (_, i) => `r${i}`))).toHaveLength(QUICK_REPLIES_MAX);
  });

  it("counts characters, not bytes", () => {
    const cut = sanitizeQuickReplies(["好".repeat(QUICK_REPLY_MAX_CHARS + 5)])[0];
    expect(Array.from(cut)).toHaveLength(QUICK_REPLY_MAX_CHARS);
  });
});
