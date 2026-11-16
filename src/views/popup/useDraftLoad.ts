// A card that comes up with what an earlier ask of the same session and question was holding when its caller went away
// (docs/specs/popup-draft-on-cancel.md D6/D7). The host matches and hands the draft out once (`popup_draft_claim`); this
// is the part that puts it into the form and tells the user about it.
//
// It only fills what is still empty: the draft can arrive after the user has started typing, and what they typed is
// never replaced. Nothing is sent and the focus does not move; afterwards the content is the user's own, like a draft
// they typed, so the sidebar's "draft" mark and the close confirmation need no special case.
import { nextTick, ref, type Ref } from "vue";
import { popupDraftClaim } from "../../lib/ipc";
import type { DraftLoad, ImageAttachment, UserAnnotation } from "../../lib/types";
import { fileName } from "./dropFeedback";

export interface DraftLoadDeps {
  /** The questions of the card: a chosen option is only taken when the card has an option with that text. */
  questions: Ref<ReadonlyArray<{ predefinedOptions: ReadonlyArray<{ text: string }> }>>;
  chosenByQ: Ref<string[][]>;
  inputByQ: Ref<string[]>;
  imagesByQ: Ref<ImageAttachment[][]>;
  replyFilesByQ: Ref<{ path: string; name: string }[][]>;
  /** Hand the notes to the annotation layer; it places them by their quotes. Returns how many it took. */
  restoreNotes: (notes: ReadonlyArray<UserAnnotation>) => number;
  /** Make a question's text box fit what it now holds. */
  growInput: (index: number) => void;
}

export function useDraftLoad(deps: DraftLoadDeps) {
  /** The line under the navbar; null once dismissed, or when nothing was loaded. */
  const draftNotice = ref<{ missingImages: number } | null>(null);

  /** Put a claimed draft into the form, filling only what is still empty. */
  function applyDraft(draft: DraftLoad): void {
    const known = deps.questions.value;
    draft.answers.forEach((answer, index) => {
      const question = known[index];
      if (!question) return;
      if ((deps.chosenByQ.value[index] ?? []).length === 0) {
        const offered = new Set(question.predefinedOptions.map(option => option.text));
        const chosen = answer.selectedOptions.filter(text => offered.has(text));
        if (chosen.length > 0) deps.chosenByQ.value[index] = chosen;
      }
      if ((deps.inputByQ.value[index] ?? "").trim() === "" && answer.userInput !== "") {
        deps.inputByQ.value[index] = answer.userInput;
      }
      if ((deps.imagesByQ.value[index] ?? []).length === 0 && answer.images.length > 0) {
        deps.imagesByQ.value[index] = [...answer.images];
      }
      if ((deps.replyFilesByQ.value[index] ?? []).length === 0 && answer.files.length > 0) {
        deps.replyFilesByQ.value[index] = answer.files.map(path => ({ path, name: fileName(path) }));
      }
    });
    if (draft.annotations.length > 0) deps.restoreNotes(draft.annotations);
    draftNotice.value = { missingImages: draft.missingImages };
    void nextTick(() => {
      draft.answers.forEach((answer, index) => {
        if (answer.userInput !== "") deps.growInput(index);
      });
    });
  }

  /**
   * Ask the host for the draft of this card and apply it. `stillHere` says whether the form still shows the request
   * the question was asked for (the answer may arrive after the card was replaced or closed). A failed claim is
   * only a draft that did not come: the card works as it always did.
   */
  async function claimDraft(requestId: string, stillHere: () => boolean): Promise<void> {
    let draft: DraftLoad | null = null;
    try {
      draft = await popupDraftClaim(requestId);
    } catch {
      return;
    }
    if (draft && stillHere()) applyDraft(draft);
  }

  function dismissDraftNotice(): void {
    draftNotice.value = null;
  }

  return { draftNotice, claimDraft, applyDraft, dismissDraftNotice };
}
