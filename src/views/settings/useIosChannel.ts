// iOS channel card state (experimental): gateway status polling, pairing session, device list
// actions and the broker connection helpers. Everything stateful about the gateway lives in the
// daemon; this module only asks it (`ios_op`) and renders what comes back.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  iosConnectionExport,
  iosConnectionImport,
  iosOp,
  iosPairQr,
  iosTest,
} from "../../lib/ipc";
import type { IosGatewayState, IosPhone } from "../../lib/types";
import { useSettingsContext } from "./context";

const IDLE_POLL_MS = 3000;
const PAIRING_POLL_MS = 1000;

interface LocalPairing {
  ticketId: string;
  code: string;
  expiresAtMs: number;
  qrSvg: string;
}

export type PairNotice = { kind: "done"; name: string } | { kind: "ended" } | { kind: "error"; text: string };

export function useIosChannel() {
  const ctx = useSettingsContext();
  const { persist } = ctx;
  const config = computed(() => ctx.config.value!);
  const ios = computed(() => config.value.channels.ios);

  const state = ref<IosGatewayState | null>(null);
  const now = ref(Date.now());
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  let clockTimer: ReturnType<typeof setInterval> | null = null;
  let disposed = false;

  const activePhones = computed<IosPhone[]>(() =>
    (state.value?.phones ?? []).filter((p) => p.state === "active"),
  );

  // ——— state polling ———

  async function refresh() {
    if (!ios.value.enabled) {
      state.value = null;
      return;
    }
    try {
      const result = await iosOp({ op: "state" });
      if (result.kind === "state") state.value = result.state;
    } catch {
      state.value = null;
    }
    settlePairing();
  }

  function schedule() {
    if (disposed) return;
    if (pollTimer) clearTimeout(pollTimer);
    const delay = pairing.value ? PAIRING_POLL_MS : IDLE_POLL_MS;
    pollTimer = setTimeout(async () => {
      await refresh();
      schedule();
    }, delay);
  }

  function refreshSoon() {
    void refresh().then(schedule);
  }

  onMounted(() => {
    clockTimer = setInterval(() => (now.value = Date.now()), 1000);
    refreshSoon();
  });
  onBeforeUnmount(() => {
    disposed = true;
    if (pollTimer) clearTimeout(pollTimer);
    if (clockTimer) clearInterval(clockTimer);
    // A pairing code left open would outlive the window that shows it.
    if (pairing.value) void iosOp({ op: "pairCancel" }).catch(() => {});
  });
  watch(
    () => ios.value.enabled,
    () => refreshSoon(),
  );

  // ——— status line ———

  const statusKind = computed<"off" | "ok" | "warn">(() => {
    if (!state.value) return "off";
    return state.value.connected ? "ok" : "warn";
  });

  // ——— pairing ———

  const pairing = ref<LocalPairing | null>(null);
  const pairBusy = ref(false);
  const pairNotice = ref<PairNotice | null>(null);
  const codeCopied = ref(false);
  let knownPhones = new Set<string>();
  let pairStartedAt = 0;

  /** The gateway's view of the session this window started (stage, confirmation code). */
  const livePairing = computed(() => {
    const live = state.value?.pairing;
    return live && pairing.value && live.ticketId === pairing.value.ticketId ? live : null;
  });

  const pairSecondsLeft = computed(() =>
    pairing.value ? Math.max(0, Math.ceil((pairing.value.expiresAtMs - now.value) / 1000)) : 0,
  );

  /** The six-digit confirmation code as "123 456". */
  const sasText = computed(() => {
    const sas = livePairing.value?.sas ?? "";
    return sas.length === 6 ? `${sas.slice(0, 3)} ${sas.slice(3)}` : sas;
  });

  async function startPair() {
    pairBusy.value = true;
    pairNotice.value = null;
    try {
      await refresh();
      knownPhones = new Set((state.value?.phones ?? []).map((p) => p.phoneId));
      const result = await iosOp({ op: "pairStart" });
      if (result.kind !== "pairing") throw new Error("unexpected reply");
      const qrSvg = await iosPairQr(result.code);
      pairStartedAt = Date.now();
      pairing.value = {
        ticketId: result.ticketId,
        code: result.code,
        expiresAtMs: result.expiresAtMs,
        qrSvg,
      };
    } catch (e) {
      pairNotice.value = { kind: "error", text: String(e) };
    } finally {
      pairBusy.value = false;
    }
    refreshSoon();
  }

  /** The gateway dropped the session this window started: success when a new phone showed up. */
  function settlePairing() {
    const local = pairing.value;
    if (!local || state.value?.pairing?.ticketId === local.ticketId) return;
    // A poll that was already in flight when the session started may predate it.
    if (Date.now() - pairStartedAt < 2000) return;
    const added = (state.value?.phones ?? []).find(
      (p) => p.state === "active" && !knownPhones.has(p.phoneId),
    );
    pairing.value = null;
    pairNotice.value = added
      ? { kind: "done", name: added.aliasName || added.name }
      : { kind: "ended" };
  }

  async function cancelPair() {
    const local = pairing.value;
    pairing.value = null;
    pairNotice.value = null;
    if (local) await iosOp({ op: "pairCancel" }).catch(() => {});
    refreshSoon();
  }

  async function decidePair(allow: boolean) {
    const local = pairing.value;
    if (!local) return;
    try {
      await iosOp({ op: "pairDecision", ticketId: local.ticketId, allow });
    } catch (e) {
      pairNotice.value = { kind: "error", text: String(e) };
    }
    refreshSoon();
  }

  async function copyPairCode() {
    if (!pairing.value) return;
    try {
      await navigator.clipboard.writeText(pairing.value.code);
    } catch {
      /* clipboard unavailable: the QR code still works */
    }
    codeCopied.value = true;
    setTimeout(() => (codeCopied.value = false), 1500);
  }

  // ——— devices ———

  const revokeTarget = ref<IosPhone | null>(null);
  const renameTarget = ref<string | null>(null);
  const renameText = ref("");

  async function confirmRevoke() {
    const target = revokeTarget.value;
    revokeTarget.value = null;
    if (!target) return;
    await iosOp({ op: "revoke", phoneId: target.phoneId }).catch(() => {});
    refreshSoon();
  }

  function beginRename(phone: IosPhone) {
    renameTarget.value = phone.phoneId;
    renameText.value = phone.aliasName || phone.name;
  }

  async function commitRename() {
    const id = renameTarget.value;
    const name = renameText.value.trim();
    renameTarget.value = null;
    if (!id || !name) return;
    await iosOp({ op: "rename", phoneId: id, name }).catch(() => {});
    refreshSoon();
  }

  // ——— broker settings helpers ———

  const testing = ref(false);
  const testMessage = ref<string | null>(null);
  const testError = ref(false);

  async function runTest() {
    testing.value = true;
    testMessage.value = null;
    const c = ios.value;
    try {
      testMessage.value = await iosTest({
        host: c.host,
        port: c.port,
        wsPort: c.wsPort,
        username: c.username,
        password: c.password,
      });
      testError.value = false;
    } catch (e) {
      testMessage.value = String(e);
      testError.value = true;
    } finally {
      testing.value = false;
    }
  }

  const importText = ref("");
  const connectionMessage = ref<string | null>(null);
  const connectionError = ref(false);

  async function importConnection(doneText: string) {
    const text = importText.value.trim();
    if (!text) return;
    try {
      const fields = await iosConnectionImport(text);
      Object.assign(ios.value, fields);
      importText.value = "";
      connectionMessage.value = doneText;
      connectionError.value = false;
      await persist();
    } catch (e) {
      connectionMessage.value = String(e);
      connectionError.value = true;
    }
  }

  async function exportConnection(doneText: string) {
    try {
      await navigator.clipboard.writeText(await iosConnectionExport());
      connectionMessage.value = doneText;
      connectionError.value = false;
    } catch (e) {
      connectionMessage.value = String(e);
      connectionError.value = true;
    }
  }

  return {
    ios,
    state,
    activePhones,
    statusKind,
    pairing,
    pairBusy,
    pairNotice,
    codeCopied,
    livePairing,
    pairSecondsLeft,
    sasText,
    startPair,
    cancelPair,
    decidePair,
    copyPairCode,
    revokeTarget,
    renameTarget,
    renameText,
    confirmRevoke,
    beginRename,
    commitRename,
    testing,
    testMessage,
    testError,
    runTest,
    importText,
    connectionMessage,
    connectionError,
    importConnection,
    exportConnection,
  };
}
