import { mount } from "@vue/test-utils";
import { nextTick, ref } from "vue";
import { afterEach, describe, expect, it, vi } from "vitest";
import { i18n } from "../../i18n";
import AnswerComposer from "./AnswerComposer.vue";
import { PopupCtxKey, type PopupContext } from "./context";

const targets: HTMLElement[] = [];

afterEach(() => {
  for (const target of targets.splice(0)) target.remove();
});

function dockTarget(): HTMLElement {
  const target = document.createElement("div");
  target.id = "popup-composer-dock-target";
  document.body.appendChild(target);
  targets.push(target);
  return target;
}

function popupContext(dockedComposerQ = ref<number | null>(null)): PopupContext {
  const inputByQ = ref([""]);
  const inputRefs: (HTMLTextAreaElement | null)[] = [];
  return {
    inputByQ,
    imagesByQ: ref([[]]),
    replyFilesByQ: ref([[]]),
    dockedComposerQ,
    expandedQ: () => true,
    setInputRef: (el: HTMLTextAreaElement | null, i: number) => {
      inputRefs[i] = el;
    },
    setComposerAnchorRef: vi.fn(),
    setComposerHomeRef: vi.fn(),
    composerAnchorStyle: () => undefined,
    setThumbsRef: vi.fn(),
    autoGrow: vi.fn(),
    activateComposer: vi.fn(),
    onTextareaFocus: vi.fn(),
    onTextareaBlur: vi.fn(),
    onComposerInput: vi.fn(),
    onComposerMouseDown: vi.fn(),
    onComposerCompositionStart: vi.fn(),
    onComposerCompositionEnd: vi.fn(),
    onUserCaretMaybeMoved: vi.fn(),
    onTextareaMouseDown: vi.fn(),
    speechSupported: ref(false),
    listening: ref(false),
    speechReady: ref(false),
    speechError: ref<string | null>(null),
    speechStatus: ref<string | null>(null),
    speechTargetQ: ref(0),
    speechHotkeyLabel: ref(""),
    speechErrorText: (value: string) => value,
    speechStatusText: (value: string) => value,
    toggleSpeech: vi.fn(),
    focusQuestionAction: vi.fn(),
    setActive: vi.fn(),
    pickFiles: vi.fn(),
    removeImage: vi.fn(),
    removeReplyFile: vi.fn(),
  } as unknown as PopupContext;
}

describe("AnswerComposer", () => {
  it("marks collapsible composers for stable compact geometry", () => {
    dockTarget();
    const wrapper = mount(AnswerComposer, {
      props: { qIndex: 0, collapsible: true },
      attachTo: document.body,
      global: {
        plugins: [i18n],
        provide: { [PopupCtxKey as symbol]: popupContext() },
      },
    });

    expect(wrapper.get(".answer-composer").classes()).toContain("is-collapsible");
    wrapper.unmount();
  });

  it("moves the same textarea node through Teleport", async () => {
    const target = dockTarget();
    const dockedComposerQ = ref<number | null>(null);
    const ctx = popupContext(dockedComposerQ);
    const wrapper = mount(AnswerComposer, {
      props: { qIndex: 0 },
      attachTo: document.body,
      global: {
        plugins: [i18n],
        provide: { [PopupCtxKey as symbol]: ctx },
      },
    });

    const textarea = wrapper.get("textarea").element as HTMLTextAreaElement;
    await wrapper.get("textarea").setValue("draft");
    dockedComposerQ.value = 0;
    await nextTick();

    expect(target.querySelector("textarea")).toBe(textarea);
    expect(textarea.value).toBe("draft");

    dockedComposerQ.value = null;
    await nextTick();
    expect(wrapper.element.querySelector("textarea")).toBe(textarea);

    wrapper.unmount();
  });
});
