<script setup lang="ts">
// iOS app channel card (experimental tab): broker connection, device list and the pairing flow.
// docs/specs/ios-channel.md §3 / §7.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";
import { useIosChannel } from "./useIosChannel";
import type { IosPhone } from "../../lib/types";
import { formatBytes, quotaShare, trafficLevel } from "./iosTraffic";

const { t } = useI18n();
const { persist, secretsPresent, SECRET_PLACEHOLDER, clearSecret } = useSettingsContext();
const {
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

// Monthly traffic as this Mac counted it, with a warning before the free quota runs out.
const traffic = computed(() => {
  const tr = state.value?.traffic;
  return tr && tr.msgsIn + tr.msgsOut > 0 ? tr : null;
});
const trafficLevelNow = computed(() => (traffic.value ? trafficLevel(traffic.value) : "ok"));
const trafficText = computed(() => {
  const tr = traffic.value;
  if (!tr) return "";
  const share = quotaShare(tr);
  return t("settings.experimental.ios.traffic", {
    out: formatBytes(tr.bytesOut),
    in: formatBytes(tr.bytesIn),
    percent: share < 0.01 ? "<1%" : `${Math.round(share * 100)}%`,
  });
});
const trafficWarning = computed(() => {
  if (trafficLevelNow.value === "full") return t("settings.experimental.ios.trafficFull");
  if (trafficLevelNow.value === "warn") return t("settings.experimental.ios.trafficWarn");
  return "";
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
      <p v-if="trafficText" class="card-desc ios-traffic" :class="trafficLevelNow">{{ trafficText }}</p>
      <p v-if="trafficWarning" class="ios-traffic-warning" :class="trafficLevelNow">{{ trafficWarning }}</p>
      <hr class="divider" />

      <p class="ios-section-title">{{ t("settings.experimental.ios.connectionTitle") }}</p>
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
      <div class="field">
        <label>{{ t("settings.experimental.ios.relayUrl") }}</label>
        <input class="input" v-model="ios.relayUrl" spellcheck="false" @change="persist" />
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
