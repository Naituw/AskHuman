import type { OptionItem, TodoEntry } from "../../lib/types";

const MAX_WHATS_NEXT_OPTIONS = 10;
/** The most todos a Stop confirmation lists (the Mac's `MAX_OPTION_TODOS`); the rest go in the question text. */
const MAX_STOP_TODOS = 10;

/**
 * How the options of a live todo queue are laid out (docs/specs/todo-list-actions.md D9–D10).
 * What's next: the suggestions, the todos, then the final end option. Stop confirmation: the todos, then its two
 * fixed options ("continue" and "end").
 */
export type LiveTodoLayout = "whatsNext" | "stop";

export interface RefreshedWhatsNextTodos {
  options: OptionItem[];
  selectedOptions: string[];
  hiddenTodos: number;
}

/**
 * Rebuild the local TODO slice of a live todo queue while keeping the question's own options (`staticOptions`:
 * the options without a todo, as the Mac made them). Serves What's next and, with `layout: "stop"`, Stop confirmations.
 */
export function refreshWhatsNextTodos(
  currentOptions: OptionItem[],
  staticOptions: OptionItem[],
  latestTodos: TodoEntry[],
  selectedOptions: string[],
  todoPrefix: string,
  attachmentBadge: (count: number) => string,
  layout: LiveTodoLayout = "whatsNext",
): RefreshedWhatsNextTodos {
  const endOption = staticOptions[staticOptions.length - 1];
  if (!endOption) {
    return { options: currentOptions, selectedOptions, hiddenTodos: 0 };
  }

  let leading: OptionItem[];
  let trailing: OptionItem[];
  let todoSlots: number;
  if (layout === "stop") {
    leading = [];
    trailing = staticOptions;
    todoSlots = MAX_STOP_TODOS;
  } else {
    const taskSlots = MAX_WHATS_NEXT_OPTIONS - 1;
    leading = staticOptions.slice(0, -1).slice(0, taskSlots);
    trailing = [endOption];
    todoSlots = taskSlots - leading.length;
  }
  const visibleTodos = latestTodos.slice(0, todoSlots);
  const todoOptions: OptionItem[] = visibleTodos.map((todo) => {
    const frozen = currentOptions.find(
      (option) => option.todoId === todo.id && selectedOptions.includes(option.text),
    );
    if (frozen) return frozen;
    return {
      text: `${todoPrefix}${todo.text}${
        todo.attachments?.length
          ? ` ${attachmentBadge(todo.attachments.length)}`
          : ""
      }`,
      recommended: false,
      todoId: todo.id,
      todoText: todo.text,
      todoAttachments: (todo.attachments ?? []).map((attachment) => ({
        id: attachment.id,
        name: attachment.name,
        path: attachment.path,
        sourcePath: attachment.sourcePath,
        storage: attachment.storage,
      })),
    };
  });
  const options = [...leading, ...todoOptions, ...trailing];

  const selectedTodoIds = new Set(
    currentOptions
      .filter((option) => option.todoId && selectedOptions.includes(option.text))
      .map((option) => option.todoId as string),
  );
  const selectedStaticTexts = new Set(
    currentOptions
      .filter((option) => !option.todoId && selectedOptions.includes(option.text))
      .map((option) => option.text),
  );
  const reconciledSelection = options
    .filter((option) =>
      option.todoId
        ? selectedTodoIds.has(option.todoId)
        : selectedStaticTexts.has(option.text),
    )
    .map((option) => option.text);

  return {
    options,
    selectedOptions: reconciledSelection,
    hiddenTodos:
      todoSlots > 0
        ? Math.max(0, latestTodos.length - visibleTodos.length)
        : 0,
  };
}

/**
 * Where the add row sits among the options of a live todo queue: right after the last todo, before the options that
 * follow the queue (What's next: the end option; Stop: "continue" and "end"). It is also the index to draw the row
 * before when the queue is empty, so a project without todos still has its add row.
 */
export function addRowPosition(options: OptionItem[], layout: LiveTodoLayout = "whatsNext"): number {
  if (layout === "stop") return options.filter((option) => option.todoId).length;
  return Math.max(0, options.length - 1);
}

export interface SelectedWhatsNextTodo {
  id: string;
  text: string;
  optionText: string;
  attachments: NonNullable<OptionItem["todoAttachments"]>;
}

/** Resolve a locally refreshed TODO selection without relying on the daemon's request snapshot. */
export function selectedWhatsNextTodos(
  options: OptionItem[],
  selectedOptions: string[],
  latestTodos: TodoEntry[],
  todoPrefix: string,
): SelectedWhatsNextTodo[] {
  return options
    .filter((candidate) => candidate.todoId && selectedOptions.includes(candidate.text))
    .map((option) => {
      const latest = latestTodos.find((todo) => todo.id === option.todoId);
      const fallbackText = option.text.startsWith(todoPrefix)
        ? option.text.slice(todoPrefix.length)
        : option.text;
      return {
        id: option.todoId!,
        text: option.todoText ?? latest?.text ?? fallbackText,
        optionText: option.text,
        attachments:
          option.todoAttachments ?? latest?.attachments?.map((attachment) => ({
            id: attachment.id,
            name: attachment.name,
            path: attachment.path,
            sourcePath: attachment.sourcePath,
            storage: attachment.storage,
          })) ?? [],
      };
    });
}
