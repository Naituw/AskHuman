// Windows and Linux have no spare title strip inside the page, so the Agent session title goes into the
// native window title instead (docs/specs/popup-session-info.md). macOS draws it inside the page and keeps
// its native title hidden, so nothing is reported there. The page only reports the session title; the Rust
// side (`app/popup_title.rs`) owns the final text.
import { watchEffect, type Ref } from "vue";
import { popupSetSessionTitle } from "../../lib/ipc";
import { isMac } from "../../lib/platform";

export function usePopupWindowTitle(sessionTitle: Readonly<Ref<string>>, active: Readonly<Ref<boolean>>): void {
  if (isMac) return;
  watchEffect(() => {
    // The shared window mounts one page per request: only the visible one speaks for the window.
    if (!active.value) return;
    void report(sessionTitle.value.trim() || null);
  });
}

async function report(title: string | null): Promise<void> {
  try {
    await popupSetSessionTitle(title);
  } catch {
    // Best effort: a window title that cannot be updated must never get in the way of answering.
  }
}
