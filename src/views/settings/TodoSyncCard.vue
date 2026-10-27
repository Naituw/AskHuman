<script setup lang="ts">
// "Todo storage" card (experimental tab): where the todos live (this Mac only / iCloud), what the
// sync is doing, the Macs taking part and the helper app that talks to iCloud.
// docs/specs/todo-sync.md §8 (`docs/assets/todo-sync-ui/settings.jpg`).
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import { describeSync, ageOf, type SyncLine } from "../todos/syncState";
import { useTodoSync } from "./useTodoSync";

const { t } = useI18n();
const {
  sync,
  status,
  now,
  busy,
  message,
  messageError,
  needsApproval,
  enabled,
  saveDeviceName,
  enable,
  disable,
  syncNow,
  clearCloud,
  resume,
  restartFromLocal,
  repair,
  openLoginItems,
} = useTodoSync();

const failed = (e: string) => t("settings.experimental.todoSync.failed", { e });
const confirmClear = ref(false);

function ageText(ms: number): string {
  if (!ms) return t("settings.experimental.todoSync.never");
  const age = ageOf(now.value - ms);
  switch (age.unit) {
    case "justNow":
      return t("todosWin.time.justNow");
    case "seconds":
      return t("todosWin.time.secondsAgo", { n: age.n });
    case "minutes":
      return t("todosWin.time.minutesAgo", { n: age.n });
    case "hours":
      return t("todosWin.time.hoursAgo", { n: age.n });
    default:
      return t("todosWin.time.daysAgo", { n: age.n });
  }
}

const line = computed<SyncLine | null>(() => describeSync(status.value, now.value));

/** The status row: the footer's wording, with the totals when all is well. */
const statusText = computed(() => {
  const s = status.value;
  const l = line.value;
  if (!s || !l) return "";
  switch (l.key) {
    case "synced":
      return t("settings.experimental.todoSync.statusSummary", {
        todos: s.todos,
        history: s.history,
        time: ageText(Math.max(s.lastPullOkMs, s.lastPushOkMs)),
      });
    case "syncing":
      return s.lastPullOkMs || s.lastPushOkMs
        ? t("settings.experimental.todoSync.syncing")
        : t("settings.experimental.todoSync.statusFirst");
    case "offline":
      return t("todosWin.sync.offline", { n: l.count ?? 0 });
    case "helperProblem":
      return `${t("todosWin.sync.helperProblem")}: ${l.detail ?? ""}`;
    case "error":
      return `${t("todosWin.sync.error")}${l.detail ? `: ${l.detail}` : ""}`;
    default:
      return t(`todosWin.sync.${l.key}`);
  }
});

const statusDot = computed(() => {
  switch (line.value?.tone) {
    case "ok":
      return "ok";
    case "busy":
      return "busy";
    case "warn":
      return "warn";
    case "error":
      return "error";
    default:
      return "off";
  }
});

const pausedText = computed(() => {
  switch (status.value?.paused) {
    case "accountChanged":
      return t("settings.experimental.todoSync.pausedAccountChanged");
    case "zoneMissing":
      return t("settings.experimental.todoSync.pausedZoneMissing");
    case "quota":
      return t("settings.experimental.todoSync.pausedQuota");
    case "notSignedIn":
      return t("settings.experimental.todoSync.pausedNotSignedIn");
    default:
      return "";
  }
});

const helperText = computed(() => {
  const h = status.value?.helper;
  if (!h) return "";
  if (h.state === "running") {
    const base = t("settings.experimental.todoSync.helperRunning", { version: h.version ?? "?" });
    return h.problem
      ? `${base} · ${t("settings.experimental.todoSync.helperProblem", { problem: h.problem })}`
      : base;
  }
  if (h.state === "incompatible") return t("settings.experimental.todoSync.helperIncompatible");
  return h.installed
    ? t("settings.experimental.todoSync.helperNotRunning")
    : t("settings.experimental.todoSync.helperNotInstalled");
});

const helperOk = computed(
  () => status.value?.helper.state === "running" && !status.value.helper.problem,
);
const canRepair = computed(() => !!status.value?.helper.installed && !helperOk.value);

async function chooseLocation(target: "local" | "icloud"): Promise<void> {
  if (target === "icloud" && !enabled.value) await enable(failed);
  if (target === "local" && enabled.value) await disable(failed);
}

async function onClear(): Promise<void> {
  confirmClear.value = false;
  await clearCloud(failed);
}
</script>

<template>
  <div v-if="status" id="todo-sync-card" class="card channel-card ts-card">
    <p class="card-title">{{ t("settings.experimental.todoSync.title") }}</p>
    <p class="card-desc">{{ t("settings.experimental.todoSync.desc") }}</p>

    <p v-if="!status.supported" class="card-desc">
      {{ t("settings.experimental.todoSync.unsupported") }}
    </p>

    <template v-else>
      <hr class="divider" />
      <div class="row ts-row">
        <span class="label">{{ t("settings.experimental.todoSync.location") }}</span>
        <span class="spacer"></span>
        <div class="segmented" role="radiogroup">
          <button
            type="button"
            class="seg"
            role="radio"
            :aria-checked="!enabled"
            :class="{ active: !enabled }"
            :disabled="!!busy"
            @click="chooseLocation('local')"
          >
            {{ t("settings.experimental.todoSync.localOnly") }}
          </button>
          <button
            type="button"
            class="seg"
            role="radio"
            :aria-checked="enabled"
            :class="{ active: enabled }"
            :disabled="!!busy"
            @click="chooseLocation('icloud')"
          >
            {{ t("settings.experimental.todoSync.icloud") }}
          </button>
        </div>
      </div>

      <template v-if="enabled">
        <div class="row ts-row">
          <span class="label">{{ t("settings.experimental.todoSync.statusLabel") }}</span>
          <span class="spacer"></span>
          <span class="ts-status" :class="statusDot">
            <span class="ts-dot" :class="statusDot"></span>{{ statusText }}
          </span>
        </div>
        <p v-if="pausedText" class="card-desc ts-paused">{{ pausedText }}</p>
        <p v-if="status.newerState" class="card-desc ts-paused">
          {{ t("settings.experimental.todoSync.newerState") }}
        </p>

        <div class="row ts-row">
          <span class="label">{{ t("settings.experimental.todoSync.deviceName") }}</span>
          <span class="spacer"></span>
          <input
            class="input ts-name"
            v-model="sync.deviceName"
            :placeholder="status.deviceName || t('settings.experimental.todoSync.deviceNamePlaceholder')"
            spellcheck="false"
            @change="saveDeviceName(failed)"
          />
        </div>
      </template>

      <div class="row ts-row">
        <span class="label">{{ t("settings.experimental.todoSync.helper") }}</span>
        <span class="spacer"></span>
        <span class="ts-helper" :class="{ bad: !helperOk }">{{ helperText }}</span>
        <button
          v-if="canRepair"
          type="button"
          class="btn ts-inline-btn"
          :disabled="!!busy"
          @click="repair(failed)"
        >
          {{ t("settings.experimental.todoSync.repair") }}
        </button>
      </div>
      <div v-if="needsApproval" class="ts-approval">
        <p class="card-desc">{{ t("settings.experimental.todoSync.repairApproval") }}</p>
        <button type="button" class="btn" @click="openLoginItems">
          {{ t("settings.experimental.todoSync.openLoginItems") }}
        </button>
      </div>

      <template v-if="enabled && status.devices.length">
        <hr class="divider" />
        <p class="ts-section-title">{{ t("settings.experimental.todoSync.devices") }}</p>
        <ul class="ts-devices">
          <li v-for="d in status.devices" :key="d.name + d.thisMac" class="ts-device">
            <span class="ts-device-name">
              {{ d.name }}
              <span v-if="d.thisMac" class="ts-me">{{ t("settings.experimental.todoSync.thisMac") }}</span>
            </span>
            <span class="ts-device-meta">
              {{ t("settings.experimental.todoSync.lastSync", { time: ageText(d.lastSyncMs) }) }}
            </span>
          </li>
        </ul>
      </template>

      <div v-if="enabled" class="row ts-actions">
        <button type="button" class="btn" :disabled="!!busy" @click="syncNow(failed)">
          {{ busy === "now" ? t("settings.experimental.todoSync.syncing") : t("settings.experimental.todoSync.syncNow") }}
        </button>
        <button type="button" class="btn" :disabled="!!busy" @click="disable(failed)">
          {{ t("settings.experimental.todoSync.stop") }}
        </button>
        <template v-if="status.paused === 'accountChanged' || status.paused === 'zoneMissing'">
          <button type="button" class="btn btn-primary" :disabled="!!busy" @click="restartFromLocal(failed)">
            {{ t("settings.experimental.todoSync.restartFromLocal") }}
          </button>
        </template>
        <button
          v-else-if="status.paused === 'quota'"
          type="button"
          class="btn btn-primary"
          :disabled="!!busy"
          @click="resume(failed)"
        >
          {{ t("settings.experimental.todoSync.resume") }}
        </button>
        <span class="spacer"></span>
        <button type="button" class="ts-danger-link" :disabled="!!busy" @click="confirmClear = true">
          {{ t("settings.experimental.todoSync.clearCloud") }}
        </button>
      </div>
      <p v-if="enabled" class="card-desc ts-hint">{{ t("settings.experimental.todoSync.stopHint") }}</p>
      <p v-if="message" class="result" :class="messageError ? 'err' : 'ok'">{{ message }}</p>
    </template>
  </div>

  <div v-if="confirmClear" class="workspace-panel-backdrop">
    <section
      class="confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      :aria-label="t('settings.experimental.todoSync.clearTitle')"
    >
      <h2>{{ t("settings.experimental.todoSync.clearTitle") }}</h2>
      <p class="confirm-note">{{ t("settings.experimental.todoSync.clearDesc") }}</p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="confirmClear = false">
          {{ t("settings.experimental.todoSync.cancel") }}
        </button>
        <button class="btn btn-primary" type="button" @click="onClear">
          {{ t("settings.experimental.todoSync.clearOk") }}
        </button>
      </div>
    </section>
  </div>
</template>

<style scoped>
.ts-card .ts-row {
  min-height: 34px;
  padding: 4px 0;
}
.ts-status {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: 12.5px;
  color: var(--text-secondary);
}
.ts-status.warn {
  color: #ff9f0a;
}
.ts-status.error {
  color: #ff453a;
}
.ts-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-tertiary, #8e8e93);
}
.ts-dot.ok {
  background: #30d158;
}
.ts-dot.busy {
  background: var(--accent);
}
.ts-dot.warn {
  background: #ff9f0a;
}
.ts-dot.error {
  background: #ff453a;
}
.ts-name {
  width: 220px;
  text-align: right;
}
.ts-helper {
  font-size: 12.5px;
  color: var(--text-secondary);
}
.ts-helper.bad {
  color: #ff453a;
}
.ts-inline-btn {
  margin-left: var(--space-2);
}
.ts-paused {
  color: #ff9f0a;
}
.ts-approval {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  margin: 2px 0 var(--space-2);
}
.ts-approval .card-desc {
  margin: 0;
  flex: 1;
}
.ts-section-title {
  margin: 0 0 var(--space-2);
  font-size: 12.5px;
  font-weight: 600;
}
.ts-devices {
  list-style: none;
  margin: 0;
  padding: 0;
}
.ts-device {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
  padding: 5px 0;
  font-size: 13px;
}
.ts-device + .ts-device {
  border-top: var(--hairline) solid var(--border);
}
.ts-me {
  margin-left: 6px;
  padding: 0 6px;
  border-radius: 999px;
  font-size: 10.5px;
  color: var(--text-secondary);
  background: var(--bg-subtle, rgba(127, 127, 127, 0.12));
}
.ts-device-meta {
  font-size: 11.5px;
  color: var(--text-secondary);
}
.ts-actions {
  margin-top: var(--space-3);
  gap: var(--space-2);
}
.ts-danger-link {
  border: none;
  background: transparent;
  color: #ff453a;
  font-size: 12.5px;
  cursor: pointer;
}
.ts-danger-link:disabled {
  opacity: 0.5;
  cursor: default;
}
.ts-hint {
  margin-top: var(--space-2);
}
</style>
