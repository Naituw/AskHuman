// Complete, delete and add on every todo list a request page shows (docs/specs/todo-list-actions.md): the folded
// "Todos" section of an ordinary question and the todo options of What's next and of a Stop confirmation.
//
// The list itself (`todos`, the options built from it, the picks) belongs to the popup core. This module holds what the
// actions need: rows in their "completing" grace, the delete that waits for its second click, the add row, and the
// keyboard rules that keep Enter / Esc typed into the add row from sending the answer or closing the window.
import { nextTick, onBeforeUnmount, onMounted, ref, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import { todosAdd, todosComplete, todosRemove } from "../../lib/ipc";
import { primaryModifierPressed } from "../../lib/platform";
import type { TodoEntry } from "../../lib/types";

/** How long a completed row keeps its place, struck through, with "Undo" the only action left (as in the Todos window). */
export const COMPLETE_GRACE_MS = 1000;
/** How long a row that was just added stays lit. */
const JUST_ADDED_MS = 1200;
/** How long the note "added, but not among the rows shown" stays. */
const NOTE_MS = 5000;

export interface TodoActionsDeps {
  /** The project the request belongs to; empty = nowhere to write. */
  projectPath: Ref<string>;
  /** The queue as the page last read it. */
  todos: Ref<TodoEntry[]>;
  /** Show this queue (the section's rows, the live options, the picks): the popup core's own routine. */
  applyTodos: (latest: TodoEntry[]) => void;
  /** Re-read the queue from the store. */
  loadTodos: () => Promise<void>;
  /** Forget the local pick of a todo that is being completed or deleted. */
  unpick: (id: string) => void;
  /** The pane being shown has a todo list with an add row (the section, also an empty one, or live options). */
  hasTodoList: Ref<boolean>;
  /** The todo is among the rows on screen (a live queue lists only so many). */
  isTodoShown: (id: string) => boolean;
  /** Open the folded section so that its add row is visible. */
  expandList: () => void;
  /** The scrolling content of the page: where the rows are looked up to be scrolled into view. */
  contentEl: Ref<HTMLElement | null>;
  openTodosWindow: () => void;
  /** Enter sends (any modifier + Enter breaks the line) instead of ⌘/Ctrl+Enter sending. */
  submitWithBareEnter: Ref<boolean>;
  /** The in-page find bar is open; it covers the navbar, so ⌘T stays out of the way. */
  findActive: Ref<boolean>;
  /** The key press is the user's own speech shortcut: it wins over ⌘T. */
  speechTaken: (event: KeyboardEvent) => boolean;
  /** Insert a newline at the caret of the focused textarea. */
  insertNewline: () => void;
  /** The answer is being sent: the actions rest. */
  submitting: Ref<boolean>;
}

interface PendingComplete {
  project: string;
  timer: ReturnType<typeof setTimeout>;
}

export function useTodoActions(deps: TodoActionsDeps) {
  const { t } = useI18n();

  // ---- Rows: complete and delete ------------------------------------------------------------------------------
  /** Rows struck through for their grace second (a new Set each change: the ref holds it shallowly). */
  const completingIds = ref<Set<string>>(new Set());
  const pending = new Map<string, PendingComplete>();
  /** The row whose ✕ was clicked once: its button now reads "Confirm delete". */
  const confirmDeleteId = ref<string | null>(null);
  /** The row that was just added (lit for a moment, kept in view). */
  const addedId = ref<string | null>(null);
  let addedTimer: ReturnType<typeof setTimeout> | undefined;
  let noteTimer: ReturnType<typeof setTimeout> | undefined;

  function setCompleting(id: string, on: boolean) {
    if (completingIds.value.has(id) === on) return;
    const next = new Set(completingIds.value);
    if (on) next.add(id);
    else next.delete(id);
    completingIds.value = next;
  }

  function isTodoCompleting(id: string | null | undefined): boolean {
    return !!id && completingIds.value.has(id);
  }

  /** ✓: the row is struck through at once; after the grace second the todo is completed (taken into history). */
  function completeTodo(id: string) {
    const project = deps.projectPath.value;
    if (!project || deps.submitting.value || pending.has(id)) return;
    confirmDeleteId.value = null;
    // A todo that is done is not an answer any more.
    deps.unpick(id);
    setCompleting(id, true);
    pending.set(id, { project, timer: setTimeout(() => void commitComplete(id), COMPLETE_GRACE_MS) });
  }

  function undoCompleteTodo(id: string) {
    const entry = pending.get(id);
    if (!entry) return;
    clearTimeout(entry.timer);
    pending.delete(id);
    setCompleting(id, false);
  }

  async function commitComplete(id: string) {
    const entry = pending.get(id);
    if (!entry) return;
    pending.delete(id);
    try {
      await todosComplete(entry.project, id);
    } catch {
      // Nothing was changed: the row simply stays.
      setCompleting(id, false);
      return;
    }
    setCompleting(id, false);
    if (entry.project !== deps.projectPath.value) return;
    deps.applyTodos(deps.todos.value.filter((todo) => todo.id !== id));
    void deps.loadTodos();
  }

  /** Commit every completion still in its grace second now (the answer is going out, or the page is being reused). */
  function flushTodoCompletes() {
    for (const [id, entry] of [...pending]) {
      clearTimeout(entry.timer);
      void commitComplete(id);
    }
  }

  /** The first click on ✕: ask for the second. */
  function askDeleteTodo(id: string) {
    if (deps.submitting.value) return;
    confirmDeleteId.value = id;
  }

  /** The second click: delete for good (there is no history to take it back from). */
  async function removeTodo(id: string) {
    if (deps.submitting.value) return;
    confirmDeleteId.value = null;
    deps.unpick(id);
    deps.applyTodos(deps.todos.value.filter((todo) => todo.id !== id));
    try {
      await todosRemove(deps.projectPath.value, id);
    } catch {
      // Best effort: the todo may already be gone from the file.
    }
    void deps.loadTodos();
  }

  /** A click anywhere but on the confirm button gives the delete up (the button itself stops its click). */
  function onDocumentClick() {
    confirmDeleteId.value = null;
  }
  onMounted(() => document.addEventListener("click", onDocumentClick));
  onBeforeUnmount(() => {
    document.removeEventListener("click", onDocumentClick);
    flushTodoCompletes();
    clearTimeout(addedTimer);
    clearTimeout(noteTimer);
  });

  function clearAdded() {
    clearTimeout(addedTimer);
    addedTimer = undefined;
    addedId.value = null;
  }

  /** Light the row that was just added and bring it into view once it is on the page. */
  function markAdded(id: string) {
    clearAdded();
    addedId.value = id;
    addedTimer = setTimeout(clearAdded, JUST_ADDED_MS);
    void nextTick(() => {
      const rows = deps.contentEl.value?.querySelectorAll<HTMLElement>("[data-todo-id]") ?? [];
      for (const row of rows) {
        if (row.dataset.todoId === id) {
          row.scrollIntoView?.({ block: "nearest" });
          break;
        }
      }
    });
  }

  // ---- The add row --------------------------------------------------------------------------------------------
  /** The add row is an input (not the dashed "Add a todo" row). */
  const addOpen = ref(false);
  const addDraft = ref("");
  const addBusy = ref(false);
  const addError = ref("");
  /** "Added, but not among the rows shown": a live queue lists only so many. */
  const addNote = ref("");
  /** The add input; the component registers it. */
  const addInputEl = ref<HTMLTextAreaElement | null>(null);

  function focusAddInput() {
    const el = addInputEl.value;
    if (!el) return;
    el.focus({ preventScroll: true });
    // The whole add row (its border, hint and error line), not just the textarea: scrolling the textarea alone leaves
    // the row's lower edge under the footer. The input is resized to its content a tick later, which looks again.
    (el.closest<HTMLElement>(".todo-add-wrap") ?? el).scrollIntoView?.({ block: "nearest" });
  }

  /** Turn the add row into the input and put the caret in it. */
  async function openTodoAdd() {
    if (deps.submitting.value) return;
    addOpen.value = true;
    addError.value = "";
    await nextTick();
    focusAddInput();
  }

  /** Back to the one-line row; what was typed stays until the page is reused. */
  function closeTodoAdd() {
    addOpen.value = false;
    addError.value = "";
  }

  function setNote(text: string) {
    clearTimeout(noteTimer);
    addNote.value = text;
    noteTimer = text ? setTimeout(() => (addNote.value = ""), NOTE_MS) : undefined;
  }

  async function submitTodoAdd() {
    const text = addDraft.value.trim();
    const project = deps.projectPath.value;
    if (!text || !project || addBusy.value || deps.submitting.value) return;
    addBusy.value = true;
    addError.value = "";
    setNote("");
    try {
      const entry = await todosAdd(project, text);
      addDraft.value = "";
      markAdded(entry.id);
      // On the page at once (the end of the queue); the store's own order follows from the re-read.
      deps.applyTodos([...deps.todos.value.filter((todo) => todo.id !== entry.id), entry]);
      deps.expandList();
      await deps.loadTodos();
      if (!deps.isTodoShown(entry.id)) {
        const place = deps.todos.value.findIndex((todo) => todo.id === entry.id) + 1;
        if (place > 0) setNote(t("popup.todos.addedHidden", { n: place }));
      }
    } catch (error) {
      addError.value = t("popup.todos.addFailed", { reason: String(error) });
    } finally {
      addBusy.value = false;
    }
    // The button or the key leaves the focus where it was; keep typing possible.
    if (addOpen.value) {
      await nextTick();
      focusAddInput();
    }
  }

  /** ⌘T: put the caret in the add row of the pane's list, or open the Todos window when the pane has none. */
  async function focusTodoAdd() {
    if (!deps.hasTodoList.value) {
      deps.openTodosWindow();
      return;
    }
    deps.expandList();
    addOpen.value = true;
    await nextTick();
    focusAddInput();
  }

  /** Forget everything about the previous request (the core is initialised once per request). */
  function resetTodoActions() {
    flushTodoCompletes();
    completingIds.value = new Set();
    confirmDeleteId.value = null;
    clearAdded();
    addOpen.value = false;
    addDraft.value = "";
    addBusy.value = false;
    addError.value = "";
    setNote("");
  }

  /**
   * Returns true when the event was consumed (the caller must stop handling it). Call it before the popup's own
   * shortcuts: Enter / Esc typed in the add input never reach "send the answer" or the window's own Esc, and ⌘T adds a
   * todo wherever the focus is.
   */
  function handleTodoKeydown(event: KeyboardEvent): boolean {
    const composing = event.isComposing || event.keyCode === 229;
    const mod = primaryModifierPressed(event);
    const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
    const anyModifier = event.metaKey || event.ctrlKey || event.shiftKey || event.altKey;

    if (mod && !event.altKey && !event.shiftKey && key === "t" && !deps.speechTaken(event)) {
      if (composing || deps.findActive.value) return false;
      event.preventDefault();
      // The caret is already in the add row: nothing to do.
      if (addOpen.value && addInputEl.value && document.activeElement === addInputEl.value) return true;
      void focusTodoAdd();
      return true;
    }

    const target = event.target;
    // A bare Enter on one of the list's own buttons (✓ ✕, the add row) presses it, and does not send the answer
    // when bare Enter is the send key. Modified Enter stays the send shortcut.
    if (
      event.key === "Enter" &&
      !anyModifier &&
      target instanceof Element &&
      target.closest(".todo-acts, .todo-add") &&
      target.tagName === "BUTTON"
    ) {
      return true;
    }

    if (!addInputEl.value || target !== addInputEl.value) return false;
    // Composing: the input method owns the key.
    if (composing) return true;
    if (event.key === "Escape") {
      event.preventDefault();
      closeTodoAdd();
      return true;
    }
    if (event.key === "Enter") {
      const primaryOnly = mod && !event.shiftKey && !event.altKey;
      // Same rules as the answer box: with bare-Enter sending, any modifier + Enter breaks the line; with
      // ⌘/Ctrl+Enter sending, every other modifier + Enter does.
      const bare = deps.submitWithBareEnter.value;
      const newline = bare ? anyModifier : anyModifier && !primaryOnly;
      const send = bare ? !anyModifier : primaryOnly;
      if (newline) {
        event.preventDefault();
        deps.insertNewline();
      } else if (send) {
        event.preventDefault();
        void submitTodoAdd();
      }
      // Otherwise (bare Enter while ⌘/Ctrl+Enter sends): the browser inserts the newline itself.
      return true;
    }
    return false;
  }

  return {
    completingTodoIds: completingIds,
    confirmDeleteTodoId: confirmDeleteId,
    addedTodoId: addedId,
    todoAddOpen: addOpen,
    todoDraft: addDraft,
    todoAdding: addBusy,
    todoAddError: addError,
    todoAddNote: addNote,
    todoAddInputEl: addInputEl,
    isTodoCompleting,
    completeTodo,
    undoCompleteTodo,
    flushTodoCompletes,
    askDeleteTodo,
    removeTodo,
    openTodoAdd,
    closeTodoAdd,
    submitTodoAdd,
    focusTodoAdd,
    resetTodoActions,
    handleTodoKeydown,
  };
}
