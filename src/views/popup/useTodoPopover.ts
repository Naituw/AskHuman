// Popup "add a todo to this project" popover (docs/specs/todo-quick-add.md): the state and actions
// behind the navbar todo button, and the keyboard rules of the open popover.
//
// The popover is KEYBOARD-MODAL. While it is open `handleKeydown` swallows every shortcut the popup
// owns (Enter / Esc / ⌘1–9 / ⌘[ ⌘] / speech / find …) except ⌘W, so a key meant for the popover can
// never submit an answer, approve a permission card or close the window (spec D7, §6.2).
import { computed, nextTick, ref, watch, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import { todosAdd } from "../../lib/ipc";
import { primaryModifierPressed } from "../../lib/platform";
import type { TodoEntry } from "../../lib/types";

/** How long the row that was just added stays highlighted. */
const JUST_ADDED_MS = 1200;

export interface TodoPopoverDeps {
  /** The project the request belongs to; empty = nothing to write to (the button opens the window). */
  projectPath: Ref<string>;
  todos: Ref<TodoEntry[]>;
  /** Re-read the project's todos (also refreshes the body todo section / whats-next options). */
  loadTodos: () => Promise<void>;
  /** Remove one todo (local state first, then the store). */
  removeTodo: (id: string) => Promise<void>;
  /** Enter sends (any modifier + Enter breaks the line) instead of ⌘/Ctrl+Enter sending. */
  submitWithBareEnter: Ref<boolean>;
  /** The in-page find bar is open; it covers the navbar buttons, so ⌘T stays out of the way. */
  findActive: Ref<boolean>;
  /** The key press is the user's own speech shortcut: it wins over ⌘T. */
  speechTaken: (event: KeyboardEvent) => boolean;
  /** Insert a newline at the caret of the focused textarea. */
  insertNewline: () => void;
  /** The popup cannot take input right now (hidden, behind another dialog, a confirm modal is up). */
  suspended: () => boolean;
}

export function useTodoPopover(deps: TodoPopoverDeps) {
  const { t } = useI18n();

  const todoPopoverOpen = ref(false);
  const todoDraft = ref("");
  const todoAdding = ref(false);
  const todoAddError = ref("");
  /** The row that was just added (highlighted for a moment, scrolled into view by the component). */
  const todoJustAddedId = ref<string | null>(null);
  /** The popover's textarea; the component registers it. */
  const todoInputEl = ref<HTMLTextAreaElement | null>(null);

  /** A request without a project has nowhere to write: its todo button opens the todos window. */
  const todoPopoverAvailable = computed(() => !!deps.projectPath.value);

  let restoreFocusEl: HTMLElement | null = null;
  let justAddedTimer: ReturnType<typeof setTimeout> | undefined;

  function clearJustAdded() {
    clearTimeout(justAddedTimer);
    justAddedTimer = undefined;
    todoJustAddedId.value = null;
  }

  async function openTodoPopover() {
    if (todoPopoverOpen.value || !todoPopoverAvailable.value || deps.suspended()) return;
    const active = document.activeElement;
    restoreFocusEl = active instanceof HTMLElement && active !== document.body ? active : null;
    todoPopoverOpen.value = true;
    todoAddError.value = "";
    void deps.loadTodos();
    await nextTick();
    todoInputEl.value?.focus({ preventScroll: true });
  }

  /** `restoreFocus` puts the focus back where it was; skipped when the focus already moved on. */
  function closeTodoPopover(restoreFocus = true) {
    if (!todoPopoverOpen.value) return;
    todoPopoverOpen.value = false;
    clearJustAdded();
    const target = restoreFocusEl;
    restoreFocusEl = null;
    if (restoreFocus && target?.isConnected) {
      void nextTick(() => target.focus({ preventScroll: true }));
    }
  }

  function toggleTodoPopover() {
    if (todoPopoverOpen.value) closeTodoPopover();
    else void openTodoPopover();
  }

  async function addTodoFromPopover() {
    const text = todoDraft.value.trim();
    const project = deps.projectPath.value;
    if (!text || !project || todoAdding.value) return;
    todoAdding.value = true;
    todoAddError.value = "";
    try {
      const entry = await todosAdd(project, text);
      todoDraft.value = "";
      clearJustAdded();
      todoJustAddedId.value = entry.id;
      justAddedTimer = setTimeout(clearJustAdded, JUST_ADDED_MS);
      await deps.loadTodos();
    } catch (error) {
      todoAddError.value = t("popup.todoPopover.addFailed", { reason: String(error) });
    } finally {
      todoAdding.value = false;
    }
    // The add button (or the shortcut) leaves the focus where it was; keep typing possible.
    if (todoPopoverOpen.value) todoInputEl.value?.focus({ preventScroll: true });
  }

  async function removeTodoFromPopover(id: string) {
    await deps.removeTodo(id);
    await deps.loadTodos();
  }

  /** Forget everything about the previous request (the core is initialised once per request). */
  function resetTodoPopover() {
    todoPopoverOpen.value = false;
    todoDraft.value = "";
    todoAddError.value = "";
    restoreFocusEl = null;
    clearJustAdded();
  }

  // A window-level dialog, a confirmation modal, or another request taking the window over closes
  // the popover; the focus already belongs to whatever took over.
  watch(deps.suspended, (suspended) => {
    if (suspended) closeTodoPopover(false);
  });

  /**
   * Returns true when the event was consumed (the caller must stop handling it). Call it before
   * any other popup shortcut.
   */
  function handleTodoPopoverKeydown(event: KeyboardEvent): boolean {
    const composing = event.isComposing || event.keyCode === 229;
    const mod = primaryModifierPressed(event);
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    const toggles = mod && !event.altKey && !event.shiftKey && key === "t" && !deps.speechTaken(event);

    if (!todoPopoverOpen.value) {
      if (!toggles || composing || deps.findActive.value || !todoPopoverAvailable.value) return false;
      event.preventDefault();
      void openTodoPopover();
      return true;
    }

    // Open: nothing below may reach the rest of the popup, whatever the focus is on.
    if (composing) return true;
    // Closing the window stays available; the popup's own ⌘W handler takes it from here.
    if (mod && key === "w") return false;
    if (toggles || event.key === "Escape") {
      event.preventDefault();
      closeTodoPopover();
      return true;
    }
    if (event.key === "Enter") {
      // Enter on a button keeps its native click; anything outside the input does nothing else.
      if (event.target !== todoInputEl.value) return true;
      const anyModifier = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;
      const primaryOnly = mod && !event.shiftKey && !event.altKey;
      // Same rules as the answer box: with bare-Enter sending, any modifier + Enter breaks the
      // line; with ⌘/Ctrl+Enter sending, every other modifier + Enter does.
      const bare = deps.submitWithBareEnter.value;
      const newline = bare ? anyModifier : anyModifier && !primaryOnly;
      const send = bare ? !anyModifier : primaryOnly;
      if (newline) {
        event.preventDefault();
        deps.insertNewline();
      } else if (send) {
        event.preventDefault();
        void addTodoFromPopover();
      }
      // Otherwise (bare Enter while ⌘/Ctrl+Enter sends): the browser inserts the newline itself.
      return true;
    }
    return true;
  }

  return {
    todoPopoverOpen,
    todoPopoverAvailable,
    todoDraft,
    todoAdding,
    todoAddError,
    todoJustAddedId,
    todoInputEl,
    openTodoPopover,
    closeTodoPopover,
    toggleTodoPopover,
    addTodoFromPopover,
    removeTodoFromPopover,
    resetTodoPopover,
    handleTodoPopoverKeydown,
  };
}
