import { computed } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  updateDismiss: vi.fn(async (_version: string) => {}),
  popupUpdateState: vi.fn(async () => ({
    available: true,
    latestVersion: "1.2.0",
    pending: false,
    applyMode: "automatic",
  })),
  listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));
vi.mock("../../lib/ipc", () => ({
  openSettings: vi.fn(async () => {}),
  popupUpdateState: mocks.popupUpdateState,
  updateApply: vi.fn(async () => {}),
  updateDismiss: mocks.updateDismiss,
  updateGetNotes: vi.fn(async () => ""),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    mocks.listeners.set(name, handler);
    return () => mocks.listeners.delete(name);
  }),
}));
vi.mock("vue-i18n", () => ({ useI18n: () => ({ t: (key: string) => key }) }));
vi.mock("../../lib/markdown", () => ({
  renderMarkdown: (text: string) => text,
  markdownReady: Promise.resolve(),
}));
import { useUpdateState } from "./useUpdateState";

async function started() {
  const state = useUpdateState({
    codeCopyLabels: computed(() => ({ copyLabel: "Copy", copiedLabel: "Copied" })),
  });
  await state.initUpdateState();
  return state;
}

describe("skipping an update version from the popup", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mocks.listeners.clear();
  });

  it("dismisses the version the daemon announced and hides the prompt", async () => {
    const state = await started();
    expect(state.updateAvailable.value).toBe(true);
    state.updatePopoverOpen.value = true;

    await state.dismissUpdateVersion();

    expect(mocks.updateDismiss).toHaveBeenCalledExactlyOnceWith("1.2.0");
    expect(state.updatePopoverOpen.value).toBe(false);
    expect(state.updateAvailable.value).toBe(false);
    state.disposeUpdateState();
  });

  it("shows the prompt again when a newer version is announced", async () => {
    const state = await started();
    await state.dismissUpdateVersion();
    expect(state.updateAvailable.value).toBe(false);

    mocks.listeners.get("update-state")?.({
      payload: { available: true, latestVersion: "1.3.0", pending: false, applyMode: "automatic" },
    });

    expect(state.updateAvailable.value).toBe(true);
    expect(state.updateLatest.value).toBe("1.3.0");
    state.disposeUpdateState();
  });

  it("keeps the prompt and says why when the dismissal cannot be saved", async () => {
    const state = await started();
    state.updatePopoverOpen.value = true;
    mocks.updateDismiss.mockRejectedValueOnce("disk full");

    await state.dismissUpdateVersion();

    expect(state.updateAvailable.value).toBe(true);
    expect(state.updatePopoverOpen.value).toBe(true);
    expect(state.updateError.value).toBe("disk full");
    state.disposeUpdateState();
  });

  it("does nothing while the update is being applied or no version is known", async () => {
    const state = await started();
    state.updating.value = true;
    await state.dismissUpdateVersion();
    state.updating.value = false;
    state.updateStarted.value = true;
    await state.dismissUpdateVersion();
    state.updateStarted.value = false;
    state.updateLatest.value = "";
    await state.dismissUpdateVersion();

    expect(mocks.updateDismiss).not.toHaveBeenCalled();
    expect(state.updateAvailable.value).toBe(true);
    state.disposeUpdateState();
  });
});
