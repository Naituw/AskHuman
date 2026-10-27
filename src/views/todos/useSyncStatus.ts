// iCloud sync state for the todos window. Nothing here is on the open path: the window paints from
// local data first, then `start()` asks for the status once; after that the status is refreshed on
// focus and on a slow heartbeat, and only while sync is on (spec todo-sync §9.9 rules 1 and 3).
import { computed, onBeforeUnmount, ref } from "vue";
import { todoSyncStatus, todoSyncTouch } from "../../lib/ipc";
import type { TodoSyncStatus } from "../../lib/types";
import { describeSync, pollDelayMs } from "./syncState";

export function useSyncStatus() {
  const status = ref<TodoSyncStatus | null>(null);
  const now = ref(Date.now());
  let timer: ReturnType<typeof setTimeout> | null = null;
  let started = false;
  let disposed = false;
  let inflight = false;

  async function refresh(): Promise<void> {
    if (inflight || disposed) return;
    inflight = true;
    try {
      status.value = await todoSyncStatus();
      now.value = Date.now();
    } catch {
      /* the strip simply keeps what it shows */
    } finally {
      inflight = false;
    }
    schedule();
  }

  function schedule(): void {
    if (disposed) return;
    if (timer) clearTimeout(timer);
    timer = null;
    const delay = pollDelayMs(status.value);
    if (delay !== null) timer = setTimeout(() => void refresh(), delay);
  }

  function onFocus(): void {
    // Pull if the last pull is stale (the backend decides; this returns at once), then look again.
    if (status.value?.enabled) void todoSyncTouch().catch(() => {});
    void refresh();
  }

  /** Call once the first frame is out. */
  function start(): void {
    if (started || disposed) return;
    started = true;
    window.addEventListener("focus", onFocus);
    void refresh();
  }

  onBeforeUnmount(() => {
    disposed = true;
    if (timer) clearTimeout(timer);
    window.removeEventListener("focus", onFocus);
  });

  const line = computed(() => describeSync(status.value, now.value));
  return { status, line, start, refresh };
}
