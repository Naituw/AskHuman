// State of the iCloud todo sync card (experimental tab): the status the backend reports, the
// actions that change it, and a poll that only runs while something can change by itself.
// Everything stateful lives in the sync service and the helper app; this module asks and renders.
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import {
  todoSyncClearCloud,
  todoSyncDisable,
  todoSyncEnable,
  todoSyncNow,
  todoSyncOpenLoginItems,
  todoSyncRepair,
  todoSyncRestartFromLocal,
  todoSyncResume,
  todoSyncStatus,
} from "../../lib/ipc";
import type { TodoSyncConfig, TodoSyncStatus } from "../../lib/types";
import { pollDelayMs } from "../todos/syncState";
import { useSettingsContext } from "./context";

export type SyncBusy = "" | "enable" | "disable" | "now" | "clear" | "repair" | "resume" | "restart";

const DEFAULT_CONFIG: TodoSyncConfig = { deviceName: "", pollMinutes: 10 };

export function useTodoSync() {
  const ctx = useSettingsContext();
  const { persist } = ctx;
  const config = computed(() => ctx.config.value!);
  // Configs saved before this feature have no section.
  if (!config.value.todoSync) config.value.todoSync = { ...DEFAULT_CONFIG };
  const sync = computed<TodoSyncConfig>(() => config.value.todoSync ?? DEFAULT_CONFIG);

  const status = ref<TodoSyncStatus | null>(null);
  const now = ref(Date.now());
  const busy = ref<SyncBusy>("");
  /** Result of the last action: shown under the buttons. */
  const message = ref("");
  const messageError = ref(false);
  /** The helper waits for the user's approval under System Settings → Login Items. */
  const needsApproval = ref(false);

  let timer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;
  let inflight = false;

  async function refresh(): Promise<void> {
    if (inflight) return;
    inflight = true;
    try {
      status.value = await todoSyncStatus();
      now.value = Date.now();
    } catch {
      /* keep what we have: the next poll tries again */
    } finally {
      inflight = false;
    }
    schedule();
  }

  function schedule(): void {
    if (disposed) return;
    if (timer) clearTimeout(timer);
    timer = null;
    // An action in flight changes the answer soon; otherwise follow the status' own cadence.
    const delay = busy.value ? 1500 : pollDelayMs(status.value);
    if (delay === null) return;
    timer = setTimeout(() => void refresh(), delay);
  }

  onMounted(() => {
    void refresh();
    window.addEventListener("focus", onFocus);
  });
  onBeforeUnmount(() => {
    disposed = true;
    if (timer) clearTimeout(timer);
    window.removeEventListener("focus", onFocus);
  });
  function onFocus(): void {
    void refresh();
  }

  function fail(error: unknown, failedText: (e: string) => string): void {
    message.value = failedText(String(error));
    messageError.value = true;
  }

  /** Run one action: keep the buttons busy meanwhile, adopt the status it returns. */
  async function run(
    kind: Exclude<SyncBusy, "">,
    action: () => Promise<TodoSyncStatus>,
    failedText: (e: string) => string,
  ): Promise<void> {
    if (busy.value) return;
    busy.value = kind;
    message.value = "";
    messageError.value = false;
    needsApproval.value = false;
    schedule();
    try {
      status.value = await action();
      now.value = Date.now();
    } catch (error) {
      fail(error, failedText);
    } finally {
      busy.value = "";
      schedule();
    }
  }

  const enabled = computed(() => status.value?.enabled === true);

  return {
    sync,
    status,
    now,
    busy,
    message,
    messageError,
    needsApproval,
    enabled,
    refresh,
    /** Name changes reach the other Macs with the next tick; ask for one right away. */
    async saveDeviceName(failedText: (e: string) => string): Promise<void> {
      await persist();
      if (enabled.value) await run("now", todoSyncNow, failedText);
    },
    enable: (failedText: (e: string) => string) => run("enable", todoSyncEnable, failedText),
    disable: (failedText: (e: string) => string) => run("disable", todoSyncDisable, failedText),
    syncNow: (failedText: (e: string) => string) => run("now", todoSyncNow, failedText),
    clearCloud: (failedText: (e: string) => string) => run("clear", todoSyncClearCloud, failedText),
    resume: (failedText: (e: string) => string) => run("resume", todoSyncResume, failedText),
    restartFromLocal: (failedText: (e: string) => string) =>
      run("restart", todoSyncRestartFromLocal, failedText),
    /** Register the helper as a login item; when macOS wants the user's approval, say so. */
    async repair(failedText: (e: string) => string): Promise<void> {
      if (busy.value) return;
      busy.value = "repair";
      message.value = "";
      messageError.value = false;
      needsApproval.value = false;
      try {
        const outcome = await todoSyncRepair();
        needsApproval.value = outcome.needsApproval;
        if (!outcome.ok && !outcome.needsApproval) {
          message.value = failedText(outcome.message || "repair failed");
          messageError.value = true;
        }
      } catch (error) {
        fail(error, failedText);
      } finally {
        busy.value = "";
        await refresh();
      }
    },
    openLoginItems: () => void todoSyncOpenLoginItems().catch(() => {}),
  };
}
