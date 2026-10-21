import { effectScope, nextTick, ref } from "vue";
import { beforeEach, describe, expect, it, vi } from "vitest";

// The popup reports its Agent session title for the native window title on Windows / Linux; macOS draws it in the page
// instead (docs/specs/popup-session-info.md). `isMac` is read when the composable module loads, so each platform loads
// a fresh copy of it.
const setTitle = vi.fn();

async function load(mac: boolean) {
  vi.resetModules();
  vi.doMock("../../lib/platform", () => ({ isMac: mac }));
  vi.doMock("../../lib/ipc", () => ({ popupSetSessionTitle: setTitle }));
  return (await import("./usePopupWindowTitle")).usePopupWindowTitle;
}

function run(use: Awaited<ReturnType<typeof load>>, title = "", active = true) {
  const sessionTitle = ref(title);
  const isActive = ref(active);
  const scope = effectScope();
  scope.run(() => use(sessionTitle, isActive));
  return { sessionTitle, isActive, stop: () => scope.stop() };
}

describe("the popup's native window title (Windows / Linux)", () => {
  beforeEach(() => {
    setTitle.mockReset();
    setTitle.mockResolvedValue(undefined);
  });

  it("reports the trimmed session title of the visible popup", async () => {
    const { stop } = run(await load(false), "  Fix the popup title \n");
    expect(setTitle).toHaveBeenCalledExactlyOnceWith("Fix the popup title");
    stop();
  });

  it("reports null when the session has no title, so the plain title comes back", async () => {
    const { stop } = run(await load(false), "   ");
    expect(setTitle).toHaveBeenCalledExactlyOnceWith(null);
    stop();
  });

  it("follows a title that arrives late and one that changes", async () => {
    const { sessionTitle, stop } = run(await load(false));
    expect(setTitle).toHaveBeenLastCalledWith(null);
    sessionTitle.value = "Late title";
    await nextTick();
    expect(setTitle).toHaveBeenLastCalledWith("Late title");
    sessionTitle.value = "Renamed";
    await nextTick();
    expect(setTitle).toHaveBeenLastCalledWith("Renamed");
    expect(setTitle).toHaveBeenCalledTimes(3);
    stop();
  });

  it("stays silent while another request is the visible one, and speaks up once it is shown", async () => {
    const { sessionTitle, isActive, stop } = run(await load(false), "Hidden request", false);
    sessionTitle.value = "Still hidden";
    await nextTick();
    expect(setTitle).not.toHaveBeenCalled();
    isActive.value = true;
    await nextTick();
    expect(setTitle).toHaveBeenCalledExactlyOnceWith("Still hidden");
    stop();
  });

  it("stops reporting when the popup page goes away", async () => {
    const { sessionTitle, stop } = run(await load(false), "First");
    stop();
    sessionTitle.value = "After unmount";
    await nextTick();
    expect(setTitle).toHaveBeenCalledExactlyOnceWith("First");
  });

  it("never reports on macOS, which draws the title in the page", async () => {
    const { sessionTitle, stop } = run(await load(true), "Fix the popup title");
    sessionTitle.value = "Renamed";
    await nextTick();
    expect(setTitle).not.toHaveBeenCalled();
    stop();
  });

  it("shrugs off a window that cannot be retitled", async () => {
    setTitle.mockRejectedValue(new Error("no such window"));
    const { stop } = run(await load(false), "Fix the popup title");
    await nextTick();
    expect(setTitle).toHaveBeenCalledOnce();
    stop();
  });
});
