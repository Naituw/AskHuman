export function settleTextareaHeightAfterBlur(
  textarea: HTMLTextAreaElement | null,
  expanded: boolean,
) {
  // Blur does not change content, so an expanded editor must keep its measured height.
  // Resetting through `height: auto` can trigger WebKit scroll anchoring between
  // pointerdown and pointerup, moving an option out from under the pointer.
  if (textarea && !expanded) textarea.style.height = "";
}
