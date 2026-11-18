// The card's side of loading what an abandoned ask left behind (docs/specs/popup-draft-on-cancel.md D6/D7): what goes
// into the form, what is never overwritten, and what the line under the navbar says.
import { nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DraftLoad, DraftLoadAnswer, ImageAttachment, UserAnnotation } from "../../lib/types";

const mocks = vi.hoisted(() => ({ claim: vi.fn() }));
vi.mock("../../lib/ipc", () => ({ popupDraftClaim: mocks.claim }));
import { useDraftLoad } from "./useDraftLoad";

const png = (name: string): ImageAttachment => ({ data: "data:image/png;base64,iVBORw0KGgo=", mediaType: "image/png", filename: name });
const answer = (patch: Partial<DraftLoadAnswer> = {}): DraftLoadAnswer => ({ selectedOptions: [], userInput: "", images: [], files: [], ...patch });
const draft = (answers: DraftLoadAnswer[], patch: Partial<DraftLoad> = {}): DraftLoad => ({ answers, annotations: [], missingImages: 0, ...patch });

function card(questions = 2) {
  const form = {
    questions: ref(Array.from({ length: questions }, () => ({ predefinedOptions: [{ text: "Yes" }, { text: "No" }] }))),
    chosenByQ: ref<string[][]>(Array.from({ length: questions }, () => [])),
    inputByQ: ref<string[]>(Array.from({ length: questions }, () => "")),
    imagesByQ: ref<ImageAttachment[][]>(Array.from({ length: questions }, () => [])),
    replyFilesByQ: ref<{ path: string; name: string }[][]>(Array.from({ length: questions }, () => [])),
    restoreNotes: vi.fn<(notes: ReadonlyArray<UserAnnotation>) => number>(() => 0),
    growInput: vi.fn<(index: number) => void>(),
  };
  return { form, load: useDraftLoad(form) };
}

describe("loading a draft into a card", () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it("puts every part of each question into that question", async () => {
    const { form, load } = card();
    load.applyDraft(draft([
      answer({ selectedOptions: ["No"], userInput: "because it is slow", images: [png("a.png")], files: ["/tmp/dir/report.txt"] }),
      answer({ userInput: "second" }),
    ], { annotations: [{ quote: "Some", note: "which one?" }] }));
    expect(form.chosenByQ.value).toEqual([["No"], []]);
    expect(form.inputByQ.value).toEqual(["because it is slow", "second"]);
    expect(form.imagesByQ.value[0]).toEqual([png("a.png")]);
    expect(form.imagesByQ.value[1]).toEqual([]);
    expect(form.replyFilesByQ.value[0]).toEqual([{ path: "/tmp/dir/report.txt", name: "report.txt" }]);
    expect(form.restoreNotes).toHaveBeenCalledWith([{ quote: "Some", note: "which one?" }]);
    expect(load.draftNotice.value).toEqual({ missingImages: 0 });
  });

  it("makes the text boxes fit what they now hold, and only those that got text", async () => {
    const { form, load } = card();
    load.applyDraft(draft([answer(), answer({ userInput: "two\nlines" })]));
    await nextTick();
    expect(form.growInput.mock.calls).toEqual([[1]]);
  });

  it("does not take notes when the draft has none", () => {
    const { form, load } = card();
    load.applyDraft(draft([answer({ userInput: "text" })]));
    expect(form.restoreNotes).not.toHaveBeenCalled();
  });

  // Spec D7: the draft may arrive after the user started typing. What they typed stays, the rest is filled.
  it("fills only what is still empty", () => {
    const { form, load } = card();
    form.inputByQ.value[0] = "typed since the card came up";
    form.chosenByQ.value[1] = ["Yes"];
    form.imagesByQ.value[1] = [png("mine.png")];
    form.replyFilesByQ.value[1] = [{ path: "/mine", name: "mine" }];
    load.applyDraft(draft([
      answer({ selectedOptions: ["No"], userInput: "old text", images: [png("old.png")], files: ["/old"] }),
      answer({ selectedOptions: ["No"], userInput: "old text 2", images: [png("old2.png")], files: ["/old2"] }),
    ]));
    // Question 1: the text is theirs; the rest was empty and fills.
    expect(form.inputByQ.value[0]).toBe("typed since the card came up");
    expect(form.chosenByQ.value[0]).toEqual(["No"]);
    expect(form.imagesByQ.value[0]).toEqual([png("old.png")]);
    expect(form.replyFilesByQ.value[0]).toEqual([{ path: "/old", name: "old" }]);
    // Question 2: options, pictures and files are theirs; only the empty text fills.
    expect(form.chosenByQ.value[1]).toEqual(["Yes"]);
    expect(form.imagesByQ.value[1]).toEqual([png("mine.png")]);
    expect(form.replyFilesByQ.value[1]).toEqual([{ path: "/mine", name: "mine" }]);
    expect(form.inputByQ.value[1]).toBe("old text 2");
    // The line is shown all the same: the draft was loaded, as far as it fit.
    expect(load.draftNotice.value).toEqual({ missingImages: 0 });
  });

  it("treats a box with only blanks in it as empty", () => {
    const { form, load } = card(1);
    form.inputByQ.value[0] = "  \n ";
    load.applyDraft(draft([answer({ userInput: "kept text" })]));
    expect(form.inputByQ.value[0]).toBe("kept text");
  });

  it("takes only the options the card offers", () => {
    const { form, load } = card(1);
    load.applyDraft(draft([answer({ selectedOptions: ["Yes", "Maybe", "No"] })]));
    expect(form.chosenByQ.value[0]).toEqual(["Yes", "No"]);
    const other = card(1);
    other.load.applyDraft(draft([answer({ selectedOptions: ["Maybe"] })]));
    expect(other.form.chosenByQ.value[0]).toEqual([]);
  });

  it("ignores answers for questions the card does not have", () => {
    const { form, load } = card(1);
    load.applyDraft(draft([answer({ userInput: "first" }), answer({ userInput: "no such question" })]));
    expect(form.inputByQ.value).toEqual(["first"]);
  });

  it("says how many pictures could not be brought back", () => {
    const { load } = card(1);
    load.applyDraft(draft([answer({ userInput: "text" })], { missingImages: 3 }));
    expect(load.draftNotice.value).toEqual({ missingImages: 3 });
  });

  it("lets the line be dismissed, and keeps the content", () => {
    const { form, load } = card(1);
    load.applyDraft(draft([answer({ userInput: "text" })]));
    load.dismissDraftNotice();
    expect(load.draftNotice.value).toBeNull();
    expect(form.inputByQ.value[0]).toBe("text");
  });
});

describe("asking the host for the draft", () => {
  beforeEach(() => { vi.clearAllMocks(); });

  it("asks for the card's own request and applies what comes", async () => {
    const { form, load } = card(1);
    mocks.claim.mockResolvedValue(draft([answer({ userInput: "from before" })]));
    await load.claimDraft("req-1", () => true);
    expect(mocks.claim).toHaveBeenCalledWith("req-1");
    expect(form.inputByQ.value[0]).toBe("from before");
    expect(load.draftNotice.value).not.toBeNull();
  });

  it("leaves the card alone when there is no draft", async () => {
    const { form, load } = card(1);
    mocks.claim.mockResolvedValue(null);
    await load.claimDraft("req-1", () => true);
    expect(form.inputByQ.value[0]).toBe("");
    expect(load.draftNotice.value).toBeNull();
  });

  it("leaves the card alone when the host cannot answer", async () => {
    const { form, load } = card(1);
    mocks.claim.mockRejectedValue(new Error("command not found"));
    await expect(load.claimDraft("req-1", () => true)).resolves.toBeUndefined();
    expect(form.inputByQ.value[0]).toBe("");
    expect(load.draftNotice.value).toBeNull();
  });

  it("does not apply a draft that arrives after the card was replaced or closed", async () => {
    const { form, load } = card(1);
    mocks.claim.mockResolvedValue(draft([answer({ userInput: "too late" })]));
    await load.claimDraft("req-1", () => false);
    expect(form.inputByQ.value[0]).toBe("");
    expect(load.draftNotice.value).toBeNull();
  });
});
