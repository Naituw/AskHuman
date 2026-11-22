// iOS App card state: gateway status polling, pairing session, device list actions and the
// broker connection helpers. Everything stateful about the gateway lives in the daemon; this
// module only asks it (`ios_op`) and renders what comes back. The card's components share one
// instance through `iosChannelContext.ts`.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import {
  iosConnectionExport,
  iosConnectionImport,
  iosDefaultRelayUrl,
  iosOp,
  iosPairQr,
  iosTest,
} from "../../lib/ipc";
import type { IosGatewayState, IosPhone, IosTestError } from "../../lib/types";
import { useSettingsContext } from "./context";
import { brokerFill, detectBroker, isHintKind, type IosBrokerId, type IosTestHintKind } from "./iosBrokers";

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

  // The push relay (docs/specs/ios-open-access.md OA9): an empty address means the public relay
  // built into this build, which only exists when `defaultRelayUrl` is set. The form offers
  // "public / own" only then; without one it is just the address field, as before.
  const defaultRelayUrl = ref("");
  // The user picked "own" and may not have typed an address yet (an empty field alone reads as "public").
  const ownRelay = ref(!!ctx.config.value?.channels.ios.relayUrl?.trim());
  const relayMode = computed<"public" | "own">(() =>
    ownRelay.value || ios.value.relayUrl.trim() !== "" ? "own" : "public",
  );

  async function setRelayMode(mode: "public" | "own") {
    ownRelay.value = mode === "own";
    if (mode === "public" && ios.value.relayUrl !== "") {
      ios.value.relayUrl = "";
      await persist();
    }
  }

  // The broker service (docs/specs/ios-open-access.md OA11). No config field records it: the host
  // name decides until the user picks one here, and then the pick stands. With no pick and no host
  // there is no service yet (`null`: no tile is selected), and "custom" is only what hints fall back to.
  const brokerPick = ref<IosBrokerId | null>(null);
  const provider = computed<IosBrokerId | null>(
    () => brokerPick.value ?? (ios.value.host.trim() ? detectBroker(ios.value.host) : null),
  );
  const broker = computed<IosBrokerId>(() => provider.value ?? "custom");

  /** A password is typed now or already stored: either way the connection has one. */
  const passwordFilled = computed(
    () => ios.value.password.trim() !== "" || ctx.secretsPresent.value.iosBrokerPassword,
  );

  /** Picking a service fills in its ports (and its host when it has one for everybody). */
  async function chooseBroker(id: IosBrokerId) {
    brokerPick.value = id;
    const fill = brokerFill(id);
    if (!fill) return;
    if (fill.host) ios.value.host = fill.host;
    ios.value.port = fill.port;
    ios.value.wsPort = fill.wsPort;
    await persist();
  }

  const state = ref<IosGatewayState | null>(null);
  // How many times the gateway answered while the channel was on (a failed ask counts: it is an
  // answer, "not running"). The card tells its first answer from the ones that follow.
  const polls = ref(0);
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
    // Turned off while the question was on its way: the answer is of no use.
    if (ios.value.enabled) polls.value++;
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
    void iosDefaultRelayUrl()
      .then((url) => {
        if (!disposed) defaultRelayUrl.value = url.trim();
      })
      .catch(() => {});
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
  // Things that work but deserve a look (the account can reach outside its own namespace): shown under
  // the result, never blocking.
  const testWarnings = ref<string[]>([]);
  // A hint for the service the test ran against, for the failures a wrong setting causes there.
  const testHint = ref<{ broker: IosBrokerId; kind: IosTestHintKind } | null>(null);

  /** A failed test arrives as an `IosTestError`; anything else (a transport failure) is just its text. */
  function asTestError(e: unknown): IosTestError {
    if (e && typeof e === "object" && typeof (e as IosTestError).message === "string") {
      const failure = e as Partial<IosTestError> & { message: string };
      return { message: failure.message, kind: failure.kind ?? "other", warnings: failure.warnings ?? [] };
    }
    return { message: String(e), kind: "other", warnings: [] };
  }

  async function runTest() {
    testing.value = true;
    testMessage.value = null;
    testWarnings.value = [];
    testHint.value = null;
    const c = ios.value;
    // The service is read now: the form may change while the test runs.
    const service = broker.value;
    try {
      const result = await iosTest({
        host: c.host,
        port: c.port,
        wsPort: c.wsPort,
        username: c.username,
        password: c.password,
        relayUrl: c.relayUrl,
      });
      testMessage.value = result.message;
      testWarnings.value = result.warnings ?? [];
      testError.value = false;
      // The gateway is about to connect with these settings: do not wait for the next poll to say so.
      refreshSoon();
    } catch (e) {
      const failure = asTestError(e);
      testMessage.value = failure.message;
      testWarnings.value = failure.warnings;
      testHint.value = isHintKind(failure.kind) ? { broker: service, kind: failure.kind } : null;
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
      // The imported setting is the truth: an empty relay in it means "public", and the host decides the service again.
      ownRelay.value = fields.relayUrl.trim() !== "";
      brokerPick.value = null;
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
    provider,
    broker,
    chooseBroker,
    passwordFilled,
    defaultRelayUrl,
    relayMode,
    setRelayMode,
    state,
    polls,
    refreshSoon,
    activePhones,
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
    testWarnings,
    testHint,
    runTest,
    importText,
    connectionMessage,
    connectionError,
    importConnection,
    exportConnection,
  };
}
