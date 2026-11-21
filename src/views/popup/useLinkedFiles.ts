// Files a text links to: the text of a request or history entry, or of a Markdown file the panel is showing.
// A click previews the file in the attachment panel like an attachment's, a right click opens the attachment
// menu, and the link is selected while its file is on show (docs/specs/popup-file-links.md).
//
// The backend owns the index space: `popup_preview_link` registers the path with the owner and answers with the
// index after the owner's attachments (§4.2). Every open goes through it — registering the same path twice
// answers the same index — so nothing here derives an index. The mirror kept here (authored path → index, index →
// file) draws the selected state and lets a click on the link that is on show collapse at once.
import { computed, shallowRef, watch, type ComputedRef, type Ref } from "vue";
import { openPath, popupPreviewLink } from "../../lib/ipc";
import { primaryModifierPressed } from "../../lib/platform";
import type { FileAttachment, LinkedFile } from "../../lib/types";

/** What activating a link needs from the click; a key press passes zeros. */
export type LinkActivation = Pick<MouseEvent, "detail" | "metaKey" | "ctrlKey">;

/** The longest chain Back walks; the oldest step is dropped first. */
export const BACK_STACK_LIMIT = 50;

/** The answer is a trust boundary of its own: anything unexpected degrades to the system opening the path. */
function parseLinked(result: unknown): LinkedFile | null {
  if (!result || typeof result !== "object") return null;
  const linked = result as Partial<{ kind: string; index: number; file: FileAttachment }>;
  if (linked.kind === "other") return { kind: "other" };
  if (linked.kind === "file" && Number.isInteger(linked.index) && (linked.index as number) >= 0
      && linked.file && typeof linked.file.path === "string") {
    return { kind: "file", index: linked.index as number, file: linked.file };
  }
  return null;
}

export function useLinkedFiles(deps: {
  /** Owner id the paths are registered with: the request id in a Popup, `history:<entry id>` in History. */
  ownerId: ComputedRef<string>;
  /** The index the panel shows, or null when it is closed or shows a reply image. */
  shown: Readonly<Ref<number | null>>;
  /** The owner's real attachment at an index, if it is one: they come first in the owner's list. */
  attachmentAt: (index: number) => FileAttachment | null;
  /** Show an index in the panel, expanding it when it is collapsed. */
  show: (index: number) => void;
  /** Collapse the panel. */
  collapse: () => void;
  /** The attachment menu for an index; shared with the attachments and the panel's ⋯. */
  showMenu: (index: number) => void;
}) {
  /** Authored path → index, for the links that were clicked. */
  const byPath = shallowRef<ReadonlyMap<string, number>>(new Map());
  /** Index → file, for the linked files only: an attachment is looked up in the attachments. */
  const files = shallowRef<ReadonlyMap<number, FileAttachment>>(new Map());
  /** The files Back returns to, oldest first. Only jumps made inside the panel are recorded. */
  const stack = shallowRef<readonly number[]>([]);
  let activation = 0;
  let menu = 0;
  let jumping = false;
  let disposed = false;

  function fileAt(index: number): FileAttachment | null {
    return deps.attachmentAt(index) ?? files.value.get(index) ?? null;
  }
  const isLinkedPreview = computed(() => deps.shown.value !== null && !deps.attachmentAt(deps.shown.value));
  const canGoBack = computed(() => stack.value.length > 0);

  /** Every authored path whose file is on show, plus the file's own path (an attachment that is also linked). */
  const activePaths = computed<ReadonlySet<string>>(() => {
    const index = deps.shown.value;
    const paths = new Set<string>();
    if (index === null) return paths;
    const file = fileAt(index);
    if (file) paths.add(file.path);
    for (const [path, at] of byPath.value) if (at === index) paths.add(path);
    return paths;
  });

  // Another owner has other files: nothing registered for the last one means anything now.
  const stopOwner = watch(deps.ownerId, () => {
    activation++; menu++;
    byPath.value = new Map(); files.value = new Map(); stack.value = [];
  }, { flush: "sync" });
  // Whatever changes the file on show other than a jump made inside the panel starts navigation over: a chip, a
  // link in the text, the menu's quick look, closing the panel.
  const stopShown = watch(deps.shown, () => { if (!jumping) stack.value = []; }, { flush: "sync" });

  function current(owner: string, token: number, latest: number): boolean {
    return !disposed && token === latest && owner === deps.ownerId.value;
  }
  function openInSystem(path: string): void {
    void openPath(path).catch(() => {});
  }
  function remember(path: string, linked: Extract<LinkedFile, { kind: "file" }>): void {
    byPath.value = new Map(byPath.value).set(path, linked.index);
    if (!deps.attachmentAt(linked.index)) files.value = new Map(files.value).set(linked.index, linked.file);
  }
  function showIndex(index: number, keepStack: boolean): void {
    jumping = keepStack;
    try { deps.show(index); } finally { jumping = false; }
  }

  /**
   * A click, or Enter, on a link to a local file. `fromPanel` is a link in the Markdown file the panel shows:
   * it navigates inside the panel and can go Back, and a link to the file already on show changes nothing.
   */
  function activate(path: string, event: LinkActivation, fromPanel = false): void {
    // Respond to the first click of a gesture; the second click of a double click does nothing.
    if (event.detail > 1) return;
    // ⌘ (macOS) / Ctrl: the original file in the system's default program; the panel stays as it is.
    if (primaryModifierPressed(event)) { openInSystem(path); return; }
    const known = byPath.value.get(path);
    if (known !== undefined && known === deps.shown.value) {
      // The link whose file is on show: like its chip, a second click collapses the panel. It is the latest
      // click, so a jump still in flight must not land after it.
      activation++;
      if (!fromPanel) deps.collapse();
      return;
    }
    const owner = deps.ownerId.value, token = ++activation;
    popupPreviewLink(owner, path).then(result => {
      // The latest click wins: one that was overtaken, or whose owner is gone, lands nowhere.
      if (!current(owner, token, activation)) return;
      const linked = parseLinked(result);
      if (!linked) throw new Error("unexpected answer");
      // A directory, a device…: nothing to preview, so the system opens it as it always did.
      if (linked.kind === "other") { openInSystem(path); return; }
      remember(path, linked);
      const shown = deps.shown.value;
      if (linked.index === shown) {
        if (!fromPanel) deps.collapse();
        return;
      }
      const inPanel = fromPanel && shown !== null;
      if (inPanel) stack.value = [...stack.value, shown].slice(-BACK_STACK_LIMIT);
      showIndex(linked.index, inPanel);
    }).catch(error => {
      if (!current(owner, token, activation)) return;
      // Registration failed (or the owner holds too many linked files): the click does what it used to.
      console.error("Linked file preview failed", error);
      openInSystem(path);
    });
  }

  /** Right click: the attachment menu, for the file the link names. A directory has no menu. */
  function showLinkMenu(path: string): void {
    const owner = deps.ownerId.value, token = ++menu;
    popupPreviewLink(owner, path).then(result => {
      if (!current(owner, token, menu)) return;
      const linked = parseLinked(result);
      if (linked?.kind !== "file") return;
      remember(path, linked);
      deps.showMenu(linked.index);
    }).catch(error => console.error("Linked file menu failed", error));
  }

  /** Back to the file the panel showed before the last jump inside it. */
  function goBack(): void {
    const previous = stack.value[stack.value.length - 1];
    if (previous === undefined) return;
    activation++; // a link click still in flight must not land after Back
    stack.value = stack.value.slice(0, -1);
    showIndex(previous, true);
  }

  function disposeLinked(): void {
    disposed = true; activation++; menu++;
    stopOwner(); stopShown();
  }

  return { fileAt, isLinkedPreview, canGoBack, activePaths, activate, showLinkMenu, goBack, disposeLinked };
}
