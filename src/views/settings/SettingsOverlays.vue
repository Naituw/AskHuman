<script setup lang="ts">
// 设置页根级弹层：「允许从手机 / IM 创建 Agent 任务」的确认 + 工作目录管理面板。
// 与实验 tab 逻辑同源（状态在 useAgentTasks），但渲染在 .settings 根层以覆盖整页。
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const {
  config,
  agentTasksConfirmOpen,
  confirmEnableAgentTasks,
  workspacePanelOpen,
  closeWorkspacePanel,
  taskSettingsBusy,
  taskSettingsMessage,
  taskWorkspaces,
  pickTaskWorkspace,
  mutateTaskWorkspace,
  toggleWorkspaceMenu,
  workspaceMenuPath,
  workspaceAgents,
  workspaceLastUsed,
} = useSettingsContext();

// The "holds the IM bot channel" note only applies when an IM bot is configured: a phone-only
// (iOS app) setup has none, so the note would be wrong there.
const imBotInUse = computed(() => {
  const channels = config.value?.channels;
  return (
    !!channels &&
    (channels.telegram.enabled ||
      channels.slack.enabled ||
      channels.dingding.enabled ||
      channels.feishu.enabled)
  );
});
</script>

<template>
  <!-- 允许「从手机 / IM 创建 Agent 任务」前的副作用确认弹层 -->
  <div v-if="agentTasksConfirmOpen" class="workspace-panel-backdrop">
    <section
      class="confirm-dialog"
      role="alertdialog"
      aria-modal="true"
      :aria-label="t('settings.agentTasks.confirmTitle')"
    >
      <h2>{{ t("settings.agentTasks.confirmTitle") }}</h2>
      <p class="confirm-intro">{{ t("settings.agentTasks.confirmIntro") }}</p>
      <ul class="confirm-list">
        <li>{{ t("settings.agentTasks.confirmKeepalive") }}</li>
        <li>{{ t("settings.agentTasks.confirmResources") }}</li>
        <li v-if="imBotInUse">{{ t("settings.agentTasks.confirmBotChannel") }}</li>
      </ul>
      <p class="confirm-note">{{ t("settings.agentTasks.confirmRevert") }}</p>
      <div class="confirm-actions">
        <button class="btn" type="button" @click="agentTasksConfirmOpen = false">
          {{ t("settings.agentTasks.confirmCancel") }}
        </button>
        <button class="btn btn-primary" type="button" @click="confirmEnableAgentTasks">
          {{ t("settings.agentTasks.confirmEnable") }}
        </button>
      </div>
    </section>
  </div>

  <div v-if="workspacePanelOpen" class="workspace-panel-backdrop">
    <section
      class="workspace-panel"
      role="dialog"
      aria-modal="true"
      :aria-label="t('settings.agentTasks.workspacePanelTitle')"
    >
      <header class="workspace-panel-toolbar">
        <button class="workspace-toolbar-done" type="button" @click="closeWorkspacePanel">
          {{ t("settings.agentTasks.done") }}
        </button>
        <h2>{{ t("settings.agentTasks.workspacePanelTitle") }}</h2>
        <button
          class="workspace-toolbar-icon"
          type="button"
          :disabled="taskSettingsBusy"
          :aria-label="t('settings.agentTasks.chooseWorkspace')"
          :title="t('settings.agentTasks.chooseWorkspace')"
          @click="pickTaskWorkspace"
        >
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M10 3.5v13M3.5 10h13" />
          </svg>
        </button>
      </header>

      <div class="workspace-panel-content">
        <div v-if="taskWorkspaces.length" class="workspace-list">
          <div
            v-for="workspace in taskWorkspaces"
            :key="workspace.path"
            class="workspace-list-row"
            :class="{ 'is-hidden': workspace.hidden }"
          >
            <span class="workspace-folder-icon" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d="M3.5 7.5h6l1.8 2h9.2v8.25a2.25 2.25 0 0 1-2.25 2.25H5.75a2.25 2.25 0 0 1-2.25-2.25V7.5Z" />
                <path d="M3.5 8V6.25A2.25 2.25 0 0 1 5.75 4h3.4l1.8 2h7.3a2.25 2.25 0 0 1 2.25 2.25V9" />
              </svg>
            </span>
            <div class="workspace-list-copy">
              <div class="workspace-list-title">
                <span>{{ workspace.label }}</span>
                <span v-if="workspace.pinned" class="workspace-status-badge">{{ t("settings.agentTasks.pinned") }}</span>
                <span v-if="workspace.hidden" class="workspace-status-badge muted">{{ t("settings.agentTasks.hidden") }}</span>
              </div>
              <span class="workspace-list-path" :title="workspace.path">{{ workspace.path }}</span>
              <span class="workspace-list-meta">
                {{ workspaceAgents(workspace) }}
                <template v-if="workspaceLastUsed(workspace)"> · {{ workspaceLastUsed(workspace) }}</template>
              </span>
            </div>
            <div class="workspace-row-menu">
              <button
                class="workspace-more-button"
                type="button"
                :aria-label="t('settings.agentTasks.workspaceActions')"
                @click.stop="toggleWorkspaceMenu(workspace.path)"
              >
                <svg viewBox="0 0 20 20" aria-hidden="true">
                  <circle cx="4" cy="10" r="1.35" />
                  <circle cx="10" cy="10" r="1.35" />
                  <circle cx="16" cy="10" r="1.35" />
                </svg>
              </button>
              <div v-if="workspaceMenuPath === workspace.path" class="workspace-menu-pop">
                <button class="menu-item" type="button" @click="mutateTaskWorkspace('pin', workspace)">
                  {{ workspace.pinned ? t("settings.agentTasks.unpin") : t("settings.agentTasks.pin") }}
                </button>
                <button class="menu-item" type="button" @click="mutateTaskWorkspace('hide', workspace)">
                  {{ workspace.hidden ? t("settings.agentTasks.show") : t("settings.agentTasks.hide") }}
                </button>
                <button class="menu-item workspace-menu-danger" type="button" @click="mutateTaskWorkspace('forget', workspace)">
                  {{ t("settings.agentTasks.forget") }}
                </button>
              </div>
            </div>
          </div>
        </div>
        <div v-else class="workspace-panel-empty">
          <span class="workspace-empty-icon" aria-hidden="true">
            <svg viewBox="0 0 24 24">
              <path d="M3.5 7.5h6l1.8 2h9.2v8.25a2.25 2.25 0 0 1-2.25 2.25H5.75a2.25 2.25 0 0 1-2.25-2.25V7.5Z" />
            </svg>
          </span>
          <p>{{ t("settings.agentTasks.noWorkspaces") }}</p>
          <span>{{ t("settings.agentTasks.noWorkspacesHint") }}</span>
        </div>
        <p v-if="taskSettingsMessage" class="result workspace-panel-result">{{ taskSettingsMessage }}</p>
      </div>
      <div
        v-if="workspaceMenuPath"
        class="workspace-menu-backdrop"
        @click="workspaceMenuPath = null"
      ></div>
    </section>
  </div>
</template>
