// The navbar todo popover's state and keyboard rules (docs/specs/todo-quick-add.md D5–D8, §6.2).
import { defineComponent, h, nextTick, ref } from "vue";
import { mount, type VueWrapper } from "@vue/test-utils";
import { createI18n } from "vue-i18n";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import en from "../../i18n/en";
import { desktopPlatform } from "../../lib/platform";
import type { TodoEntry } from "../../lib/types";
import { useTodoPopover } from "./useTodoPopover";

const mocks = vi.hoisted(() => ({ todosAdd: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ todosAdd: mocks.todosAdd }));

const PRIMARY: KeyboardEventInit = desktopPlatform === "mac" ? { metaKey: true } : { ctrlKey: true };

function entry(id: string, text = `todo ${id}`): TodoEntry {
  return { id, text, createdAtMs: 1 };
}

interface SetupOptions {
  project?: string;
  bare?: boolean;
}

function setup(options: SetupOptions = {}) {
  const projectPath = ref(options.project ?? "/w/app");
  const todos = ref<TodoEntry[]>([]);
  const submitWithBareEnter = ref(options.bare ?? false);
  const findActive = ref(false);
  const blocked = ref(false);
  const speechOwnsT = ref(false);
  const loadTodos = vi.fn(async () => {});
  const removeTodo = vi.fn(async (id: string) => {
    todos.value = todos.value.filter((todo) => todo.id !== id);
  });
  const insertNewline = vi.fn();

  let api!: ReturnType<typeof useTodoPopover>;
  const wrapper: VueWrapper = mount(
    defineComponent({
      setup() {
        api = useTodoPopover({
          projectPath,
          todos,
          loadTodos,
          removeTodo,
          submitWithBareEnter,
          findActive,
          speechTaken: () => speechOwnsT.value,
          insertNewline,
          suspended: () => blocked.value,
        });
        return () => h("div");
      },
    }),
    { global: { plugins: [createI18n({ legacy: false, locale: "en", messages: { en } })] } },
  );

  // The component registers its textarea; the tests stand in for it with a real, focusable one.
  const input = document.createElement("textarea");
  document.body.appendChild(input);
  api.todoInputEl.value = input;

  /** Dispatch a keydown at `target` through the handler, the way the window listener would. */
  function press(init: KeyboardEventInit, target: EventTarget = document.body) {
    const event = new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...init });
    let consumed: boolean | undefined;
    const listener = (e: Event) => {
      consumed = api.handleTodoPopoverKeydown(e as KeyboardEvent);
    };
    window.addEventListener("keydown", listener);
    target.dispatchEvent(event);
    window.removeEventListener("keydown", listener);
    return { consumed: consumed as boolean, prevented: event.defaultPrevented };
  }

  return {
    api, wrapper, input, projectPath, todos, submitWithBareEnter, findActive, blocked, speechOwnsT,
    loadTodos, removeTodo, insertNewline, press,
  };
}

describe("useTodoPopover", () => {
  const wrappers: VueWrapper[] = [];
  const track = <T extends { wrapper: VueWrapper }>(s: T) => {
    wrappers.push(s.wrapper);
    return s;
  };

  beforeEach(() => {
    vi.clearAllMocks();
    mocks.todosAdd.mockImplementation(async (_project: string, text: string) => entry("new", text));
  });

  afterEach(() => {
    vi.useRealTimers();
    wrappers.splice(0).forEach((w) => w.unmount());
    document.body.innerHTML = "";
  });

  describe("opening with ⌘/Ctrl+T", () => {
    it("opens, reloads the todos and focuses the input", async () => {
      const s = track(setup());
      const result = s.press({ key: "t", ...PRIMARY });
      expect(result).toEqual({ consumed: true, prevented: true });
      expect(s.api.todoPopoverOpen.value).toBe(true);
      expect(s.loadTodos).toHaveBeenCalledTimes(1);
      await nextTick();
      expect(document.activeElement).toBe(s.input);
    });

    it("toggles closed again and hands the focus back to where it was", async () => {
      const s = track(setup());
      const answer = document.createElement("textarea");
      document.body.appendChild(answer);
      answer.focus();
      s.press({ key: "t", ...PRIMARY }, answer);
      await nextTick();
      expect(document.activeElement).toBe(s.input);

      expect(s.press({ key: "t", ...PRIMARY }, s.input)).toEqual({ consumed: true, prevented: true });
      expect(s.api.todoPopoverOpen.value).toBe(false);
      await nextTick();
      expect(document.activeElement).toBe(answer);
    });

    it("lets the key through when the request has no project", () => {
      const s = track(setup({ project: "" }));
      expect(s.api.todoPopoverAvailable.value).toBe(false);
      expect(s.press({ key: "t", ...PRIMARY })).toEqual({ consumed: false, prevented: false });
      expect(s.api.todoPopoverOpen.value).toBe(false);
    });

    it("stays out of the way while the in-page find bar is open", () => {
      const s = track(setup());
      s.findActive.value = true;
      expect(s.press({ key: "t", ...PRIMARY })).toEqual({ consumed: false, prevented: false });
      expect(s.api.todoPopoverOpen.value).toBe(false);
    });

    it("yields to the user's speech shortcut, also while open", () => {
      const s = track(setup());
      s.speechOwnsT.value = true;
      expect(s.press({ key: "t", ...PRIMARY })).toEqual({ consumed: false, prevented: false });
      expect(s.api.todoPopoverOpen.value).toBe(false);

      s.speechOwnsT.value = false;
      s.press({ key: "t", ...PRIMARY });
      expect(s.api.todoPopoverOpen.value).toBe(true);
      s.speechOwnsT.value = true;
      // Swallowed, but it neither closes the popover nor reaches the speech toggle.
      expect(s.press({ key: "t", ...PRIMARY }, s.input)).toEqual({ consumed: true, prevented: false });
      expect(s.api.todoPopoverOpen.value).toBe(true);
    });

    it("ignores other chords and typing", () => {
      const s = track(setup());
      expect(s.press({ key: "t" }).consumed).toBe(false);
      expect(s.press({ key: "t", shiftKey: true, ...PRIMARY }).consumed).toBe(false);
      expect(s.press({ key: "t", altKey: true, ...PRIMARY }).consumed).toBe(false);
      expect(s.press({ key: "f", ...PRIMARY }).consumed).toBe(false);
      expect(s.api.todoPopoverOpen.value).toBe(false);
    });

    it("does not open in the middle of an IME composition", () => {
      const s = track(setup());
      expect(s.press({ key: "t", isComposing: true, ...PRIMARY }).consumed).toBe(false);
      expect(s.press({ key: "t", keyCode: 229, ...PRIMARY }).consumed).toBe(false);
      expect(s.api.todoPopoverOpen.value).toBe(false);
    });

    it("does not open while the popup cannot take input", async () => {
      const s = track(setup());
      s.blocked.value = true;
      s.press({ key: "t", ...PRIMARY });
      await nextTick();
      expect(s.api.todoPopoverOpen.value).toBe(false);
      expect(s.loadTodos).not.toHaveBeenCalled();
    });

    it("closes when the popup stops taking input, without taking the focus back", async () => {
      const s = track(setup());
      const answer = document.createElement("textarea");
      document.body.appendChild(answer);
      answer.focus();
      s.press({ key: "t", ...PRIMARY }, answer);
      await nextTick();
      const other = document.createElement("button");
      document.body.appendChild(other);
      other.focus();
      s.blocked.value = true;
      await nextTick();
      expect(s.api.todoPopoverOpen.value).toBe(false);
      await nextTick();
      expect(document.activeElement).toBe(other);
    });
  });

  describe("while open the keyboard belongs to the popover", () => {
    async function opened(options: SetupOptions = {}) {
      const s = track(setup(options));
      s.press({ key: "t", ...PRIMARY });
      await nextTick();
      return s;
    }

    it("closes on Esc and consumes it", async () => {
      const s = await opened();
      expect(s.press({ key: "Escape" }, s.input)).toEqual({ consumed: true, prevented: true });
      expect(s.api.todoPopoverOpen.value).toBe(false);
    });

    it("lets ⌘/Ctrl+W through to close the window", async () => {
      const s = await opened();
      expect(s.press({ key: "w", ...PRIMARY }, s.input)).toEqual({ consumed: false, prevented: false });
      expect(s.api.todoPopoverOpen.value).toBe(true);
    });

    it("swallows every other shortcut without blocking typing", async () => {
      const s = await opened();
      for (const init of [
        { key: "1", ...PRIMARY },
        { key: "]", ...PRIMARY },
        { key: "f", ...PRIMARY },
        { key: "g", ...PRIMARY },
        { key: "ArrowDown" },
        { key: "a" },
        { key: "Tab" },
      ]) {
        // Not prevented: the character still reaches the input and Tab still moves the focus.
        expect(s.press(init, s.input), JSON.stringify(init)).toEqual({ consumed: true, prevented: false });
      }
      expect(s.api.todoPopoverOpen.value).toBe(true);
    });

    it("swallows everything during an IME composition", async () => {
      const s = await opened();
      expect(s.press({ key: "Enter", isComposing: true }, s.input)).toEqual({ consumed: true, prevented: false });
      expect(s.press({ key: "Enter", keyCode: 229 }, s.input)).toEqual({ consumed: true, prevented: false });
      expect(mocks.todosAdd).not.toHaveBeenCalled();
    });

    describe("Enter in the input, ⌘/Ctrl+Enter sends", () => {
      it("sends on ⌘/Ctrl+Enter", async () => {
        const s = await opened();
        s.api.todoDraft.value = "  fix flaky test  ";
        const result = s.press({ key: "Enter", ...PRIMARY }, s.input);
        expect(result).toEqual({ consumed: true, prevented: true });
        expect(mocks.todosAdd).toHaveBeenCalledWith("/w/app", "fix flaky test");
      });

      it("leaves a bare Enter to the browser's own newline", async () => {
        const s = await opened();
        s.api.todoDraft.value = "x";
        expect(s.press({ key: "Enter" }, s.input)).toEqual({ consumed: true, prevented: false });
        expect(mocks.todosAdd).not.toHaveBeenCalled();
        expect(s.insertNewline).not.toHaveBeenCalled();
      });

      it("breaks the line on Shift+Enter", async () => {
        const s = await opened();
        expect(s.press({ key: "Enter", shiftKey: true }, s.input)).toEqual({ consumed: true, prevented: true });
        expect(s.insertNewline).toHaveBeenCalledTimes(1);
        expect(mocks.todosAdd).not.toHaveBeenCalled();
      });
    });

    describe("Enter in the input, bare Enter sends", () => {
      it("sends on a bare Enter", async () => {
        const s = await opened({ bare: true });
        s.api.todoDraft.value = "ship it";
        expect(s.press({ key: "Enter" }, s.input)).toEqual({ consumed: true, prevented: true });
        expect(mocks.todosAdd).toHaveBeenCalledWith("/w/app", "ship it");
      });

      it("breaks the line on any modifier + Enter", async () => {
        const s = await opened({ bare: true });
        s.api.todoDraft.value = "ship it";
        for (const init of [{ shiftKey: true }, { altKey: true }, PRIMARY]) {
          expect(s.press({ key: "Enter", ...init }, s.input)).toEqual({ consumed: true, prevented: true });
        }
        expect(s.insertNewline).toHaveBeenCalledTimes(3);
        expect(mocks.todosAdd).not.toHaveBeenCalled();
      });
    });

    it("keeps Enter on a button for its own click", async () => {
      const s = await opened({ bare: true });
      s.api.todoDraft.value = "ship it";
      const button = document.createElement("button");
      document.body.appendChild(button);
      expect(s.press({ key: "Enter" }, button)).toEqual({ consumed: true, prevented: false });
      expect(mocks.todosAdd).not.toHaveBeenCalled();
    });
  });

  describe("adding", () => {
    it("trims the text, clears the draft, reloads and flashes the new row", async () => {
      vi.useFakeTimers();
      const s = track(setup());
      s.api.todoDraft.value = "  write the docs \n";
      await s.api.addTodoFromPopover();
      expect(mocks.todosAdd).toHaveBeenCalledTimes(1);
      expect(mocks.todosAdd).toHaveBeenCalledWith("/w/app", "write the docs");
      expect(s.api.todoDraft.value).toBe("");
      expect(s.api.todoAddError.value).toBe("");
      expect(s.loadTodos).toHaveBeenCalledTimes(1);
      expect(s.api.todoJustAddedId.value).toBe("new");
      vi.advanceTimersByTime(1300);
      expect(s.api.todoJustAddedId.value).toBeNull();
    });

    it("does nothing for blank text or without a project", async () => {
      const s = track(setup());
      for (const draft of ["", "   ", "\n\t"]) {
        s.api.todoDraft.value = draft;
        await s.api.addTodoFromPopover();
      }
      s.projectPath.value = "";
      s.api.todoDraft.value = "real text";
      await s.api.addTodoFromPopover();
      expect(mocks.todosAdd).not.toHaveBeenCalled();
      expect(s.api.todoDraft.value).toBe("real text");
    });

    it("keeps the draft and says why when the write fails", async () => {
      mocks.todosAdd.mockRejectedValueOnce(new Error("disk is full"));
      const s = track(setup());
      s.api.todoDraft.value = "will fail";
      await s.api.addTodoFromPopover();
      expect(s.api.todoDraft.value).toBe("will fail");
      expect(s.api.todoAddError.value).toContain("disk is full");
      expect(s.api.todoAdding.value).toBe(false);
      expect(s.loadTodos).not.toHaveBeenCalled();

      // The next attempt starts clean and can succeed.
      await s.api.addTodoFromPopover();
      expect(s.api.todoAddError.value).toBe("");
      expect(s.api.todoDraft.value).toBe("");
    });

    it("ignores a second add while the first is in flight", async () => {
      let resolve!: (value: TodoEntry) => void;
      mocks.todosAdd.mockImplementationOnce(() => new Promise<TodoEntry>((r) => (resolve = r)));
      const s = track(setup());
      s.api.todoDraft.value = "once";
      const first = s.api.addTodoFromPopover();
      expect(s.api.todoAdding.value).toBe(true);
      await s.api.addTodoFromPopover();
      expect(mocks.todosAdd).toHaveBeenCalledTimes(1);
      resolve(entry("a", "once"));
      await first;
      expect(s.api.todoAdding.value).toBe(false);
    });

    it("keeps typing possible after an add", async () => {
      const s = track(setup());
      s.press({ key: "t", ...PRIMARY });
      await nextTick();
      const outside = document.createElement("button");
      document.body.appendChild(outside);
      outside.focus();
      s.api.todoDraft.value = "next";
      await s.api.addTodoFromPopover();
      expect(document.activeElement).toBe(s.input);
    });
  });

  it("deletes through the host and reloads", async () => {
    const s = track(setup());
    s.todos.value = [entry("a"), entry("b")];
    await s.api.removeTodoFromPopover("a");
    expect(s.removeTodo).toHaveBeenCalledWith("a");
    expect(s.todos.value.map((todo) => todo.id)).toEqual(["b"]);
    expect(s.loadTodos).toHaveBeenCalledTimes(1);
  });

  it("keeps the draft while the popover is closed and reopened", async () => {
    const s = track(setup());
    s.press({ key: "t", ...PRIMARY });
    s.api.todoDraft.value = "half-written";
    s.press({ key: "Escape" }, s.input);
    expect(s.api.todoPopoverOpen.value).toBe(false);
    s.press({ key: "t", ...PRIMARY });
    expect(s.api.todoPopoverOpen.value).toBe(true);
    expect(s.api.todoDraft.value).toBe("half-written");
  });

  it("forgets everything when the next request takes the window over", async () => {
    const s = track(setup());
    s.press({ key: "t", ...PRIMARY });
    s.api.todoDraft.value = "old request's draft";
    s.api.todoAddError.value = "old error";
    s.api.resetTodoPopover();
    expect(s.api.todoPopoverOpen.value).toBe(false);
    expect(s.api.todoDraft.value).toBe("");
    expect(s.api.todoAddError.value).toBe("");
    expect(s.api.todoJustAddedId.value).toBeNull();
  });
});
