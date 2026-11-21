<script setup lang="ts">
// iOS app channel card (experimental tab): broker connection, device list and the pairing flow.
// docs/specs/ios-channel.md §3 / §7. The app's "New task" needs the Agent-task switch, which sits
// in the card as a row of its own (AgentTasksSwitchRow).
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import AgentTasksSwitchRow from "./AgentTasksSwitchRow.vue";
import { useSettingsContext } from "./context";
import { useIosChannel } from "./useIosChannel";
import type { IosPhone } from "../../lib/types";
import { openPath } from "../../lib/ipc";
import { IOS_BROKERS, brokerGuideUrl, brokerPreset, type IosBrokerId } from "./iosBrokers";
import {
  ALERT_AFTER_PRESETS,
  alertAfterChoices,
  idleDuration,
  splitDuration,
  type SplitDuration,
} from "./iosAlertTiming";
import { aggregateTraffic, brokerEstimate, hasCompleteTrafficHistory, formatBytes, trafficReminder } from "./iosTraffic";

const { t, locale } = useI18n();
const { persist, secretsPresent, SECRET_PLACEHOLDER, clearSecret } = useSettingsContext();
const {
  ios,
  broker,
  chooseBroker,
  defaultRelayUrl,
  relayMode,
  setRelayMode,
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
  testWarnings,
  testHint,
  runTest,
  importText,
  connectionMessage,
  connectionError,
  importConnection,
  exportConnection,
} = useIosChannel();

const statusText = computed(() => {
  const s = state.value;
  if (!s) return t("settings.experimental.ios.statusNotRunning");
  if (s.connected) {
    return t("settings.experimental.ios.statusConnected", { via: s.via ?? "?" });
  }
  const base = t("settings.experimental.ios.statusDisconnected");
  return s.lastError ? `${base}: ${s.lastError}` : base;
});

// Known Mac counters converge through paired phones.
const traffic = computed(() => {
  const local = state.value?.traffic;
  const records = state.value?.usageRecords ?? [];
  const tr = records.length && local ? aggregateTraffic(records, local.month) : local;
  return tr && tr.msgsIn + tr.msgsOut > 0 ? tr : null;
});
const completeTrafficHistory = computed(() => {
  const records = state.value?.usageRecords ?? [];
  return records.length > 0 && !!traffic.value && hasCompleteTrafficHistory(records, traffic.value.month);
});
const trafficText = computed(() => {
  const tr = traffic.value;
  if (!tr) return "";
  return t(completeTrafficHistory.value ? "settings.experimental.ios.trafficAggregate" : "settings.experimental.ios.trafficPartial", {
    total: formatBytes(brokerEstimate(tr)),
    count: state.value?.usageRecords?.length ?? 1,
    out: formatBytes(tr.bytesOut),
    in: formatBytes(tr.bytesIn),
  });
});
// Only counts, and says nothing of the broker's plan: close to each GB, or past it (same rule as the phone's Inbox).
const trafficNote = computed(() => {
  const reminder = traffic.value ? trafficReminder(brokerEstimate(traffic.value)) : null;
  if (!reminder) return "";
  return t(reminder.reached ? "settings.experimental.ios.trafficOver" : "settings.experimental.ios.trafficNear", { gb: reminder.gigabytes });
});

// The service the broker belongs to (docs/specs/ios-open-access.md OA11): a short set of points per service,
// and the wiki page that has the steps.
const brokerPoints = computed(() => brokerPreset(broker.value).points);

function openBrokerGuide() {
  void openPath(brokerGuideUrl(broker.value, String(locale.value).startsWith("zh")));
}

// Push timing (docs/specs/ios-alert-timing.md): the wait before an unseen question is pushed.
function durationText(d: SplitDuration): string {
  return t(`settings.experimental.ios.duration.${d.unit}`, { n: d.n });
}

const alertAfterOptions = computed(() =>
  alertAfterChoices(ios.value.alertAfterSecs).map((secs) => {
    if (secs === 0) return { secs, label: t("settings.experimental.ios.alertAfterNow") };
    const text = durationText(splitDuration(secs));
    return {
      secs,
      label: ALERT_AFTER_PRESETS.includes(secs)
        ? text
        : t("settings.experimental.ios.alertAfterCustom", { value: text }),
    };
  }),
);

const macIdleText = computed(() => {
  const secs = state.value?.macIdleSecs;
  if (secs === undefined || secs === null) return "";
  return t("settings.experimental.ios.macIdle", { time: durationText(idleDuration(secs)) });
});

const pairNoticeText = computed(() => {
  const n = pairNotice.value;
  if (!n) return "";
  if (n.kind === "done") return t("settings.experimental.ios.pairDone", { name: n.name });
  if (n.kind === "ended") return t("settings.experimental.ios.pairEnded");
  return n.text;
});

function phoneName(phone: IosPhone): string {
  return phone.aliasName || phone.name;
}

function lastSeenText(phone: IosPhone): string {
  if (phone.online) return t("settings.experimental.ios.online");
  if (!phone.lastSeenAt) return t("settings.experimental.ios.never");
  return t("settings.experimental.ios.lastSeen", {
    time: new Date(phone.lastSeenAt).toLocaleString(),
  });
}

function pushWarning(phone: IosPhone): string {
  if (phone.push === "gone") return t("settings.experimental.ios.pushGone");
  if (phone.push === "error") return t("settings.experimental.ios.pushError");
  return "";
}
</script>

<template>
  <div class="card channel-card ios-card">
    <div class="row">
      <p class="card-title">{{ t("settings.experimental.ios.title") }}</p>
      <span class="spacer"></span>
      <label class="switch">
        <input type="checkbox" v-model="ios.enabled" @change="persist" />
        <span class="track"></span>
      </label>
    </div>
    <p class="card-desc">{{ t("settings.experimental.ios.desc") }}</p>

    <template v-if="ios.enabled">
      <p class="badge ios-status">
        <span class="dot" :class="statusKind === 'ok' ? 'on' : 'off'"></span>
        {{ statusText }}
      </p>
      <p v-if="trafficText" class="card-desc ios-traffic">{{ trafficText }}</p>
      <details v-if="state?.usageRecords?.length" class="card-desc ios-traffic-details">
        <summary>{{ t("settings.experimental.ios.trafficDetails") }}</summary>
        <p v-for="record in state.usageRecords" :key="`${record.deviceKind ?? 'mac'}|${record.macId}`">
          {{ record.deviceKind === 'phone' ? t("settings.experimental.ios.trafficMobile") : 'Mac' }} · {{ record.macName || record.macId.slice(0, 8) }} · {{ formatBytes(brokerEstimate(record)) }} ·
          {{ t("settings.experimental.ios.trafficUpdated", { time: new Date(record.updatedAt).toLocaleString() }) }}
          <template v-if="state.trafficSyncedAt?.[`${record.broker}|${record.month}|${record.deviceKind ?? 'mac'}|${record.macId}`]">
            · {{ t("settings.experimental.ios.trafficSynced", { time: new Date(state.trafficSyncedAt[`${record.broker}|${record.month}|${record.deviceKind ?? 'mac'}|${record.macId}`]!).toLocaleString() }) }}
          </template>
        </p>
        <p v-if="!completeTrafficHistory">{{ t("settings.experimental.ios.trafficIncomplete") }}</p>
        <p>{{ t("settings.experimental.ios.trafficCoverage") }}</p>
      </details>
      <p v-if="state?.trafficStorageIssue" class="card-desc warn">{{ t("settings.experimental.ios.trafficStorageIssue") }}</p>
      <p v-if="trafficNote" class="card-desc ios-traffic-note">{{ trafficNote }}</p>
      <!-- The app's "New task" needs the Agent-task switch on this Mac: it is right here, where the phone is set up. -->
      <hr class="divider" />
      <AgentTasksSwitchRow />
      <hr class="divider" />

      <p class="ios-section-title">{{ t("settings.experimental.ios.connectionTitle") }}</p>
      <div class="field ios-broker">
        <label>{{ t("settings.experimental.ios.brokerLabel") }}</label>
        <select
          class="select ios-broker-select"
          :value="broker"
          @change="chooseBroker(($event.target as HTMLSelectElement).value as IosBrokerId)"
        >
          <option v-for="b in IOS_BROKERS" :key="b.id" :value="b.id">
            {{ t(`settings.experimental.ios.broker.${b.id}.name`) }}
          </option>
        </select>
        <ul class="ios-broker-points">
          <li v-for="key in brokerPoints" :key="key">
            {{ t(`settings.experimental.ios.broker.${broker}.${key}`) }}
          </li>
        </ul>
        <a class="link ios-broker-guide" href="#" @click.prevent="openBrokerGuide">
          {{ t("settings.experimental.ios.brokerGuide") }} ↗
        </a>
      </div>
      <div class="field">
        <label>{{ t("settings.experimental.ios.host") }}</label>
        <input class="input" v-model="ios.host" spellcheck="false" @change="persist" />
      </div>
      <div class="ios-ports">
        <div class="field">
          <label>{{ t("settings.experimental.ios.port") }}</label>
          <input class="input" type="number" v-model.number="ios.port" @change="persist" />
        </div>
        <div class="field">
          <label>{{ t("settings.experimental.ios.wsPort") }}</label>
          <input class="input" type="number" v-model.number="ios.wsPort" @change="persist" />
        </div>
      </div>
      <div class="field">
        <label>{{ t("settings.experimental.ios.username") }}</label>
        <input class="input" v-model="ios.username" spellcheck="false" @change="persist" />
      </div>
      <div class="field">
        <label>{{ t("settings.experimental.ios.password") }}</label>
        <div class="row">
          <input
            class="input"
            style="flex: 1"
            type="password"
            :placeholder="secretsPresent.iosBrokerPassword ? SECRET_PLACEHOLDER : ''"
            v-model="ios.password"
            @change="persist"
          />
          <button
            v-if="secretsPresent.iosBrokerPassword"
            class="btn"
            type="button"
            @click="clearSecret('iosBroker')"
          >
            {{ t("settings.channels.clearSecret") }}
          </button>
        </div>
      </div>
      <div class="field ios-relay">
        <label>{{ t(defaultRelayUrl ? "settings.experimental.ios.relayTitle" : "settings.experimental.ios.relayUrl") }}</label>
        <!-- Only a build with a public relay offers the choice; otherwise it is just the address. -->
        <select
          v-if="defaultRelayUrl"
          class="select ios-relay-mode"
          :value="relayMode"
          @change="setRelayMode(($event.target as HTMLSelectElement).value === 'own' ? 'own' : 'public')"
        >
          <option value="public">{{ t("settings.experimental.ios.relayPublic") }}</option>
          <option value="own">{{ t("settings.experimental.ios.relayOwn") }}</option>
        </select>
        <p v-if="defaultRelayUrl && relayMode === 'public'" class="card-desc ios-relay-note">
          {{ t("settings.experimental.ios.relayPublicNote") }}
        </p>
        <input
          v-if="!defaultRelayUrl || relayMode === 'own'"
          class="input"
          v-model="ios.relayUrl"
          spellcheck="false"
          :placeholder="defaultRelayUrl ? t('settings.experimental.ios.relayOwnPlaceholder') : ''"
          @change="persist"
        />
      </div>
      <div class="field">
        <label>{{ t("settings.experimental.ios.macName") }}</label>
        <input
          class="input"
          v-model="ios.macName"
          :placeholder="t('settings.experimental.ios.macNamePlaceholder')"
          @change="persist"
        />
      </div>
      <div class="row">
        <button class="btn" type="button" :disabled="testing" @click="runTest">
          {{ testing ? t("settings.channels.testing") : t("settings.channels.testConnection") }}
        </button>
        <button class="btn" type="button" @click="exportConnection(t('settings.experimental.ios.exportDone'))">
          {{ t("settings.experimental.ios.exportButton") }}
        </button>
        <span class="spacer"></span>
      </div>
      <p v-if="testMessage" class="result" :class="testError ? 'err' : 'ok'">
        {{ testMessage }}
      </p>
      <p v-if="testHint" class="ios-test-hint">
        {{ t(`settings.experimental.ios.broker.${testHint.broker}.hint.${testHint.kind}`) }}
      </p>
      <p v-for="warning in testWarnings" :key="warning" class="result warn">⚠ {{ warning }}</p>
      <div class="field ios-import">
        <label>{{ t("settings.experimental.ios.importLabel") }}</label>
        <div class="row">
          <input
            class="input"
            style="flex: 1"
            v-model="importText"
            spellcheck="false"
            :placeholder="t('settings.experimental.ios.importPlaceholder')"
            @keydown.enter.prevent="importConnection(t('settings.experimental.ios.importDone'))"
          />
          <button
            class="btn"
            type="button"
            :disabled="!importText.trim()"
            @click="importConnection(t('settings.experimental.ios.importDone'))"
          >
            {{ t("settings.experimental.ios.importButton") }}
          </button>
        </div>
      </div>
      <p v-if="connectionMessage" class="result" :class="connectionError ? 'err' : 'ok'">
        {{ connectionMessage }}
      </p>

      <hr class="divider" />
      <p class="ios-section-title">{{ t("settings.experimental.ios.alertTimingTitle") }}</p>
      <div class="row ios-alert-after">
        <span class="label">{{ t("settings.experimental.ios.alertAfterLabel") }}</span>
        <span class="spacer"></span>
        <select class="select" v-model.number="ios.alertAfterSecs" @change="persist">
          <option v-for="option in alertAfterOptions" :key="option.secs" :value="option.secs">
            {{ option.label }}
          </option>
        </select>
      </div>
      <p class="card-desc">{{ t("settings.experimental.ios.alertAfterHint") }}</p>
      <p v-if="macIdleText" class="card-desc ios-idle">{{ macIdleText }}</p>

      <hr class="divider" />
      <div class="row">
        <p class="ios-section-title">{{ t("settings.experimental.ios.devicesTitle") }}</p>
        <span class="spacer"></span>
        <button
          class="btn"
          type="button"
          :disabled="pairBusy || !!pairing || !state"
          @click="startPair"
        >
          {{
            pairBusy
              ? t("settings.experimental.ios.pairStarting")
              : t("settings.experimental.ios.pairButton")
          }}
        </button>
      </div>

      <div v-if="pairing" class="ios-pair">
        <div class="ios-pair-qr" v-html="pairing.qrSvg"></div>
        <div class="ios-pair-side">
          <template v-if="!livePairing || livePairing.stage === 'waiting'">
            <p class="card-desc">{{ t("settings.experimental.ios.pairScan") }}</p>
            <p class="card-desc hint">
              {{ t("settings.experimental.ios.pairExpires", { seconds: pairSecondsLeft }) }}
            </p>
            <div class="row">
              <button class="btn" type="button" @click="copyPairCode">
                {{
                  codeCopied
                    ? t("settings.experimental.ios.pairCopied")
                    : t("settings.experimental.ios.pairCopy")
                }}
              </button>
              <button class="btn" type="button" @click="cancelPair">
                {{ t("settings.experimental.ios.pairCancel") }}
              </button>
            </div>
          </template>
          <template v-else-if="livePairing.stage === 'pending'">
            <p class="ios-pair-title">
              {{
                t("settings.experimental.ios.pairConfirmTitle", {
                  name: livePairing.phoneName || "iPhone",
                })
              }}
            </p>
            <p class="card-desc">{{ t("settings.experimental.ios.pairConfirmDesc") }}</p>
            <p class="ios-sas">{{ sasText }}</p>
            <div class="row">
              <button class="btn btn-primary" type="button" @click="decidePair(true)">
                {{ t("settings.experimental.ios.pairAllow") }}
              </button>
              <button class="btn" type="button" @click="decidePair(false)">
                {{ t("settings.experimental.ios.pairDeny") }}
              </button>
            </div>
          </template>
          <template v-else>
            <p class="card-desc">{{ t("settings.experimental.ios.pairFinishing") }}</p>
          </template>
        </div>
      </div>
      <p
        v-if="pairNotice"
        class="result"
        :class="pairNotice.kind === 'error' ? 'err' : 'ok'"
      >
        {{ pairNoticeText }}
      </p>

      <p v-if="!activePhones.length && !pairing" class="card-desc">
        {{ t("settings.experimental.ios.noDevices") }}
      </p>
      <ul v-else-if="activePhones.length" class="ios-devices">
        <li v-for="phone in activePhones" :key="phone.phoneId" class="ios-device">
          <span class="dot" :class="phone.online ? 'on' : 'off'"></span>
          <div class="ios-device-main">
            <template v-if="renameTarget === phone.phoneId">
              <input
                class="input"
                v-model="renameText"
                @keydown.enter.prevent="commitRename"
                @keydown.esc.prevent="renameTarget = null"
                @blur="commitRename"
              />
            </template>
            <p v-else class="ios-device-name">{{ phoneName(phone) }}</p>
            <p class="ios-device-meta">{{ lastSeenText(phone) }}</p>
            <p v-if="pushWarning(phone)" class="ios-device-meta warn">{{ pushWarning(phone) }}</p>
          </div>
          <button class="btn" type="button" @click="beginRename(phone)">
            {{ t("settings.experimental.ios.rename") }}
          </button>
          <button class="btn" type="button" @click="revokeTarget = phone">
            {{ t("settings.experimental.ios.revoke") }}
          </button>
        </li>
      </ul>
    </template>
  </div>

  <div v-if="revokeTarget" class="workspace-panel-backdrop">
    <section
      class="confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      :aria-label="t('settings.experimental.ios.revokeTitle', { name: phoneName(revokeTarget) })"
    >
      <h2>{{ t("settings.experimental.ios.revokeTitle", { name: phoneName(revokeTarget) }) }}</h2>
      <p class="confirm-note">{{ t("settings.experimental.ios.revokeDesc") }}</p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="revokeTarget = null">
          {{ t("settings.experimental.ios.cancel") }}
        </button>
        <button class="btn btn-primary" type="button" @click="confirmRevoke">
          {{ t("settings.experimental.ios.revokeConfirm") }}
        </button>
      </div>
    </section>
  </div>
</template>
