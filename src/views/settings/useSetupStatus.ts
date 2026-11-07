// The setup wizard's status as the settings window needs it: the banner, the wizard card and the
// menu-bar "launch at login is not enabled" row all read this one copy. It refreshes when the
// window mounts, when it regains focus, and when the backend says something changed
// (`setup-status-changed`), so a mode switch made elsewhere clears the hints by itself.
import { onBeforeUnmount, onMounted, ref } from "vue";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { setupLoginItemEnable, setupSkip, setupStatus } from "../../lib/ipc";
import type { SetupStatus } from "../../lib/types";

export function useSetupStatus() {
  const setupState = ref<SetupStatus | null>(null);
  const loginItemBusy = ref(false);
  const loginItemError = ref<string | null>(null);

  async function refreshSetupStatus() {
    try {
      setupState.value = await setupStatus();
    } catch {
      /* keep the last known status; a hint that cannot be decided is simply not shown */
    }
  }

  /** The banner's close button: the same as skipping the Agent step in the wizard. */
  async function dismissSetupBanner() {
    try {
      await setupSkip("agents");
    } catch {
      /* the banner stays if the decision could not be written */
    }
    await refreshSetupStatus();
  }

  /** "Launch at login is not enabled yet" → Enable. */
  async function enableLoginItem() {
    if (loginItemBusy.value) return;
    loginItemBusy.value = true;
    loginItemError.value = null;
    try {
      await setupLoginItemEnable();
    } catch (e) {
      loginItemError.value = String(e);
    } finally {
      loginItemBusy.value = false;
    }
    await refreshSetupStatus();
  }

  const onFocus = () => void refreshSetupStatus();
  let unlisten: UnlistenFn | null = null;
  let unmounted = false;

  onMounted(async () => {
    void refreshSetupStatus();
    window.addEventListener("focus", onFocus);
    const off = await listen("setup-status-changed", () => void refreshSetupStatus());
    if (unmounted) off();
    else unlisten = off;
  });
  onBeforeUnmount(() => {
    unmounted = true;
    window.removeEventListener("focus", onFocus);
    unlisten?.();
  });

  return {
    setupState,
    refreshSetupStatus,
    dismissSetupBanner,
    enableLoginItem,
    loginItemBusy,
    loginItemError,
  };
}
