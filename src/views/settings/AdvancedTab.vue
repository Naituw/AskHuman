<script setup lang="ts">
// Advanced: daemon lifecycle, on-demand IM delivery, Agent task launch, and permission grants.
// Only the task-launch card is limited to platforms with a supported terminal.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import BrandBadge from "../setup/BrandBadge.vue";
import { useSettingsContext } from "./context";
import CursorMaxCard from "./CursorMaxCard.vue";
import PermissionRulesCard from "./PermissionRulesCard.vue";
import {
  readinessIssues,
  readinessState,
  startBlockers,
  startsOutsideTheCli,
  type ReadinessIssue,
  type ReadinessState,
} from "./readiness";

const { t } = useI18n();
const ctx = useSettingsContext();
const {
  isWindows,
  supportsAgentTasks,
  persist,
  changeDaemonLifecycle,
  toggleAgentTasks,
  testAgentTaskTerminal,
  taskSettingsBusy,
  taskSettingsMessage,
  refreshAgentTaskSettings,
  taskReadiness,
  taskWorkspaces,
  openReadinessIssue,
  openWorkspacePanel,
} = ctx;
// 父组件仅在 config 加载后渲染本 tab，这里可安全断言非空。
const config = computed(() => ctx.config.value!);

// What each Agent still lacks and the state of its row, worked out once per change rather than once
// per use in the template. The row's state is the backend's verdict (the one the phone acts on), so
// Codex's app and Cursor's Agents window count: `outside` marks an Agent that starts without its CLI,
// whose CLI columns are then not needed; `blockers` explain an Agent with two routes that has none.
const readinessRows = computed(() =>
  taskReadiness.value.map((item) => {
    const state = readinessState(item);
    return {
      item,
      state,
      issues: readinessIssues(item),
      outside: startsOutsideTheCli(item),
      blockers:
        state === "notReady" && item.launch && (item.kind === "codex" || item.kind === "cursor")
          ? startBlockers(item, t)
          : [],
    };
  }),
);
const hasUnsupported = computed(() => readinessRows.value.some((row) => row.state === "unsupported"));
const hasOutside = computed(() => readinessRows.value.some((row) => row.outside));
const STATE_TEXT: Record<ReadinessState, string> = {
  ready: "settings.agentTasks.ready",
  notReady: "settings.agentTasks.notReady",
  unsupported: "settings.agentTasks.unsupported",
};
/** The tag of an Agent that starts outside its CLI says where, so a ready tag never hides the route. */
function tagText(row: { item: { kind: string }; state: ReadinessState; outside: boolean }): string {
  if (!row.outside) return t(STATE_TEXT[row.state]);
  return t(
    row.item.kind === "codex"
      ? "settings.agentTasks.readyCodexApp"
      : "settings.agentTasks.readyCursorWindow",
  );
}
/** The CLI's own conditions: not needed by an Agent that starts in its app or window. */
const CLI_ONLY: ReadinessIssue[] = ["binary", "lifecycle"];
const STATE_TONE: Record<ReadinessState, string> = { ready: "ok", notReady: "warn", unsupported: "muted" };
const CONDITIONS: ReadinessIssue[] = ["binary", "lifecycle", "integration"];
const COLUMN_TEXT: Record<ReadinessIssue, string> = {
  binary: "settings.agentTasks.colBinary",
  lifecycle: "settings.agentTasks.colLifecycle",
  integration: "settings.agentTasks.colIntegration",
};
const ISSUE_TEXT: Record<ReadinessIssue, string> = {
  binary: "settings.agentTasks.issueBinary",
  lifecycle: "settings.agentTasks.issueLifecycle",
  integration: "settings.agentTasks.issueIntegration",
};
</script>

<template>
  <!-- 守护进程生命周期（默认按活动启动/空闲退出；保活=常驻+开机自启） -->
  <div class="card">
    <p class="card-title">
      {{ t("settings.experimental.daemonLifecycleTitle") }}
    </p>
    <div class="row">
      <span class="label">{{
        t("settings.experimental.daemonLifecycleLabel")
      }}</span>
      <span class="spacer"></span>
      <div class="segmented">
        <button
          :class="{ active: config.general.daemonLifecycle === 'activity' }"
          :disabled="config.agentTasks.enabled"
          @click="changeDaemonLifecycle('activity')"
        >
          {{ t("settings.experimental.daemonLifecycleActivity") }}
        </button>
        <button
          :class="{ active: config.general.daemonLifecycle === 'keepalive' }"
          :disabled="config.agentTasks.enabled"
          @click="changeDaemonLifecycle('keepalive')"
        >
          {{ t("settings.experimental.daemonLifecycleKeepalive") }}
        </button>
      </div>
    </div>
    <!-- 「允许从手机 / IM 创建 Agent 任务」依赖保活：功能开启期间锁定本控件，并就地说明原因，
         避免用户改了不生效以为是 bug（save_settings 会静默强制回 keepalive）。 -->
    <p v-if="config.agentTasks.enabled" class="card-desc warn">
      {{ t("settings.experimental.daemonLifecycleLockedByTasks") }}
    </p>
    <p class="card-desc">
      {{ t("settings.experimental.daemonLifecycleHint") }}
    </p>
  </div>

  <!-- IM 渠道按需发送（归入「高级」Tab；配置键仍为 autoActivation） -->
  <div class="card">
    <div class="row">
      <p class="card-title">
        {{ t("settings.channels.autoActivationTitle") }}
      </p>
      <span class="spacer"></span>
      <label class="switch">
        <input
          type="checkbox"
          v-model="config.channels.autoActivation"
          @change="persist"
        />
        <span class="track"></span>
      </label>
    </div>
    <p class="card-desc">
      {{ t("settings.channels.autoActivationDesc") }}
    </p>
    <p class="card-desc hint">
      {{ t("settings.channels.autoActivationLifecycleHint") }}
    </p>
    <!-- 子开关：自动结束 watch（缩进以示为「按需发送」子项；仅父开时可用，父关置灰禁用） -->
    <div
      class="sub-setting"
      :style="{ opacity: config.channels.autoActivation ? 1 : 0.5 }"
    >
      <div class="row">
        <span class="label">
          {{ t("settings.channels.autoEndWatchTitle") }}
        </span>
        <span class="spacer"></span>
        <label class="switch">
          <input
            type="checkbox"
            v-model="config.channels.autoEndWatch"
            :disabled="!config.channels.autoActivation"
            @change="persist"
          />
          <span class="track"></span>
        </label>
      </div>
      <p class="card-desc">
        {{ t("settings.channels.autoEndWatchDesc") }}
      </p>
    </div>
  </div>

  <!-- IM Agent task launch is available with Terminal.app or Windows Terminal. -->
  <div v-if="supportsAgentTasks" class="card">
    <div class="row">
      <div class="col">
        <p class="card-title">{{ t("settings.agentTasks.title") }}</p>
        <p class="card-desc">{{ t("settings.agentTasks.description") }}</p>
      </div>
      <span class="spacer"></span>
      <label class="switch">
        <input
          type="checkbox"
          v-model="config.agentTasks.enabled"
          @change="toggleAgentTasks"
        />
        <span class="track"></span>
      </label>
    </div>
    <template v-if="config.agentTasks.enabled">
      <hr class="divider" />
      <div class="row">
        <span class="label">{{ t("settings.agentTasks.permission") }}</span>
        <span class="spacer"></span>
        <select class="select" v-model="config.agentTasks.permissionPrompt" @change="persist">
          <option value="ask">{{ t("settings.agentTasks.permissionAsk") }}</option>
          <option value="agent-default">{{ t("settings.agentTasks.permissionDefault") }}</option>
          <option value="yolo">{{ t("settings.agentTasks.permissionYolo") }}</option>
        </select>
      </div>
      <p v-if="config.agentTasks.permissionPrompt === 'yolo'" class="result err">
        {{ t("settings.agentTasks.yoloWarning") }}
      </p>
      <hr class="divider" />
      <div class="row">
        <span class="label">{{ isWindows ? "Windows Terminal" : "Terminal.app" }}</span>
        <span class="spacer"></span>
        <button class="btn" type="button" @click="testAgentTaskTerminal">
          {{ t("settings.agentTasks.testTerminal") }}
        </button>
        <button class="btn" type="button" :disabled="taskSettingsBusy" @click="refreshAgentTaskSettings(true)">
          {{ t("settings.agentTasks.refresh") }}
        </button>
      </div>
      <hr class="divider" />
      <p class="label">{{ t("settings.agentTasks.readiness") }}</p>
      <div
        v-if="taskSettingsBusy && taskReadiness.length === 0"
        class="settings-section-loading"
      >
        <span class="permission-state-spinner" aria-hidden="true"></span>
        <p>{{ t("common.loading") }}</p>
      </div>
      <!-- A matrix: one row per Agent (badge, name, overall tag), one fixed column per condition. A met
           condition is a check; an unmet one is an orange cross that goes to where it is fixed. An Agent
           that cannot start tasks at all has nothing to fix: a grey tag and dashes, no crosses. An Agent
           that starts in its app or window needs no CLI: its tag says where, its CLI columns are dashes. -->
      <div v-if="readinessRows.length > 0" class="readiness-matrix" data-testid="readiness-matrix">
        <div class="readiness-row readiness-head" aria-hidden="true">
          <span class="readiness-agent"></span>
          <span v-for="cond in CONDITIONS" :key="cond" class="readiness-cell">{{ t(COLUMN_TEXT[cond]) }}</span>
        </div>
        <div
          v-for="row in readinessRows"
          :key="row.item.kind"
          class="readiness-item"
          :data-testid="`readiness-${row.item.kind}`"
        >
          <div class="readiness-row">
            <div class="readiness-agent">
              <BrandBadge :id="row.item.kind" :label="row.item.label" />
              <span class="label">{{ row.item.label }}</span>
              <span
                class="status-tag"
                :class="STATE_TONE[row.state]"
                :data-testid="`readiness-tag-${row.item.kind}`"
              >{{ tagText(row) }}</span>
            </div>
            <span v-for="cond in CONDITIONS" :key="cond" class="readiness-cell">
              <span v-if="row.state === 'unsupported'" class="readiness-na" aria-hidden="true">–</span>
              <span
                v-else-if="row.outside && CLI_ONLY.includes(cond) && row.issues.includes(cond)"
                class="readiness-na"
                role="img"
                :aria-label="`${row.item.label}: ${t(COLUMN_TEXT[cond])} · ${t('settings.agentTasks.cliNotNeeded')}`"
                :title="t('settings.agentTasks.cliNotNeeded')"
                :data-testid="`readiness-skip-${row.item.kind}-${cond}`"
              >–</span>
              <button
                v-else-if="row.issues.includes(cond)"
                type="button"
                class="readiness-fix"
                :aria-label="`${row.item.label}: ${t(ISSUE_TEXT[cond])}`"
                :title="`${t(ISSUE_TEXT[cond])} · ${cond === 'binary' ? t('settings.agentTasks.openInstallDocs') : t('settings.agentTasks.goToSetting')}`"
                :data-testid="`readiness-issue-${row.item.kind}-${cond}`"
                @click="openReadinessIssue(row.item.kind, cond)"
              >
                <svg viewBox="0 0 16 16" aria-hidden="true">
                  <path d="M5 5l6 6M11 5l-6 6" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" />
                </svg>
              </button>
              <svg
                v-else
                class="readiness-ok"
                viewBox="0 0 16 16"
                role="img"
                :aria-label="`${row.item.label}: ${t(COLUMN_TEXT[cond])}`"
              >
                <path d="M3.5 8.5l3 3 6-7" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" />
              </svg>
            </span>
          </div>
          <!-- Why an Agent with two routes (CLI and app / window) has neither: the crosses only cover the CLI's side. -->
          <p
            v-if="row.blockers.length > 0"
            class="card-desc readiness-blockers"
            :data-testid="`readiness-blockers-${row.item.kind}`"
          >{{ row.blockers.join(t("desktop.reasonSeparator")) }}</p>
        </div>
      </div>
      <p v-if="readinessRows.length > 0" class="card-desc readiness-hint">
        <span class="readiness-hint-line">{{ t("settings.agentTasks.readinessHint") }}</span>
        <span v-if="hasOutside" class="readiness-hint-line">{{ t("settings.agentTasks.outsideHint") }}</span>
        <span v-if="hasUnsupported" class="readiness-hint-line">{{ t("settings.agentTasks.unsupportedHint") }}</span>
      </p>
      <hr class="divider" />
      <div class="row">
        <div class="col">
          <span class="label">{{ t("settings.agentTasks.workspaces") }}</span>
          <span
            v-if="taskSettingsBusy && taskWorkspaces.length === 0"
            class="card-desc"
          >{{ t("common.loading") }}</span>
          <span v-else class="card-desc">{{ t("settings.agentTasks.workspaceCount", { n: taskWorkspaces.length }) }}</span>
        </div>
        <span class="spacer"></span>
        <button class="btn" type="button" @click="openWorkspacePanel">
          {{ t("settings.agentTasks.manageWorkspaces") }}
        </button>
      </div>
      <p v-if="taskSettingsMessage" class="result">{{ taskSettingsMessage }}</p>
    </template>
  </div>

  <!-- Cursor 任务的 Max 控制（spec cursor-max-control §6）：与 IM 任务开关无关，GUI／iOS 新建任务同样受它管 -->
  <CursorMaxCard v-if="supportsAgentTasks" />

  <!-- Codex 权限授权管理（spec codex-permission-remember §6.3） -->
  <PermissionRulesCard />
</template>
