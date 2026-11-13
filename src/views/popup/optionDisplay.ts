import type { OptionItem } from "../../lib/types";

/**
 * Keep the stored option value intact while hiding the legacy todo display prefix. `liveTodoOptions`: the question's
 * todo options are the project's live queue (What's next, a Stop confirmation), whose rows show the todo's own text.
 */
export function optionDisplayText(
  option: OptionItem,
  liveTodoOptions: boolean,
  todoPrefix: string,
): string {
  if (!liveTodoOptions || !option.todoId || !option.text.startsWith(todoPrefix)) {
    return option.text;
  }
  return option.todoText ?? option.text.slice(todoPrefix.length);
}
