<script setup lang="ts">
// 「Agent 集成」tab：自动集成（每家一张卡，CLI|MCP|未集成三态）+ 手动集成（参考提示词、
// MCP 配置示例）。每张卡的标题行常驻（标志、名称、状态标签、接入方式），其余各行折叠在「详情」里
// （docs/specs/settings-agents-channels-cards.md）。
import { isMac } from "../../lib/platform";
import type { AgentId } from "../../lib/types";
import BrandBadge from "../setup/BrandBadge.vue";
import CodexDesktopSettings from "./CodexDesktopSettings.vue";
import CursorDesktopSettings from "./CursorDesktopSettings.vue";
import CollaborationStyleEditor from "./CollaborationStyleEditor.vue";
import { useI18n } from "vue-i18n";
import { agentTags, hasDetails, type AgentCardInput } from "./agentCard";
import { useSettingsContext } from "./context";

const { t } = useI18n();
const {
  config,
  revealLabel,
  prompt,
  promptCopied,
  promptVariant,
  promptAgent,
  promptSupportsCli,
  promptSupportsMcp,
  setPromptAgent,
  collabBusy,
  collabError,
  collabDefaults,
  collabMessage,
  collabResults,
  collaborationSummary,
  saveAgentCollaborationStyle,
  retryCollaborationStyle,
  AGENTS,
  modes,
  integrationLoading,
  piVersionLoading,
  modeBusy,
  modeMessage,
  modeReadError,
  refreshMode,
  modeError,
  setMode,
  togglePermission,
  toggleLifecycle,
  toggleStop,
  toggleDelegated,
  toggleAskQuestion,
  permissionBlockedText,
  updateArtifact,
  updateSummary,
  updateAllBusy,
  updateAll,
  openMenuKey,
  toggleOpenMenu,
  closeOpenMenu,
  revealFile,
  openFile,
  setPromptVariant,
  mcpExampleJson,
  mcpExampleToml,
  mcpJsonCopied,
  mcpTomlCopied,
  copyMcpExample,
  copyPrompt,
  settingsTargetHighlight,
  agentExpanded,
  toggleAgentDetails,
  openAgentDetails,
} = useSettingsContext();

function cardInput(id: AgentId): AgentCardInput {
  return {
    id,
    status: modes.value[id],
    readError: modeReadError?.value?.[id] === true,
    versionLoading: id === "pi" && piVersionLoading.value,
  };
}
const tagsOf = (id: AgentId) => agentTags(cardInput(id));
/** An integrated card folds its rows into "Details"; a not-integrated one is only its header. */
const detailsOf = (id: AgentId) => hasDetails(cardInput(id));
/**
 * The result of the last action on a card sits under its header, unless the card is open: then it
 * sits at the bottom of the details, next to the rows the action came from.
 */
const messageUnderHeader = (id: AgentId) =>
  !!modeMessage.value[id] && !(detailsOf(id) && agentExpanded.value[id]);
</script>

<template>
  <div
    v-if="openMenuKey"
    class="menu-backdrop"
    @click="closeOpenMenu"
  ></div>
  <p class="section-intro">{{ t("settings.integration.overviewDesc") }}</p>

  <div v-if="config" class="card collab-style-card">
    <CollaborationStyleEditor
      :general="config.general" :defaults="collabDefaults"
      :busy="collabBusy.default" :error="collabError.default"
      :message="collabMessage.default" :results="collabResults.default"
      @save="(style, text) => saveAgentCollaborationStyle(null, style, text)"
      @retry="retryCollaborationStyle"
    />
  </div>

  <div class="integration-manual">
  <!-- 手动集成：参考提示词（CLI / MCP 双版本 + MCP 配置示例） -->
  <p class="section-title">{{ t("settings.integration.manualTitle") }}</p>
  <p class="section-intro">{{ t("settings.integration.manualLifecycleHint") }}</p>
  <div class="card">
    <div class="row">
      <p class="card-title">{{ t("settings.integration.promptTitle") }}</p>
      <span class="spacer"></span>
      <select class="input prompt-agent-choice" :aria-label="t('settings.integration.collabPreviewAgent')"
        :value="promptAgent" @change="setPromptAgent(($event.target as HTMLSelectElement).value as AgentId | '')">
        <option value="">{{ t("settings.integration.collabPreviewDefault") }}</option>
        <option v-for="agent in AGENTS" :key="agent.id" :value="agent.id">{{ agent.title }}</option>
      </select>
      <div class="segmented">
        <button
          type="button"
          class="seg"
          :disabled="promptSupportsCli === false"
          :class="{ active: promptVariant === 'cli' }"
          @click="setPromptVariant('cli')"
        >
          {{ t("settings.integration.modeCli") }}
        </button>
        <button
          type="button"
          class="seg"
          :disabled="promptSupportsMcp === false"
          :class="{ active: promptVariant === 'mcp' }"
          @click="setPromptVariant('mcp')"
        >
          {{ t("settings.integration.modeMcp") }}
        </button>
      </div>
      <button class="btn" type="button" @click="copyPrompt">
        {{
          promptCopied
            ? t("settings.integration.copied")
            : t("settings.integration.copy")
        }}
      </button>
    </div>
    <pre class="code-area">{{ prompt }}</pre>

    <template v-if="promptVariant === 'mcp'">
      <hr class="divider" />
      <p class="card-desc agent-hint">
        {{ t("settings.integration.mcpExampleHint") }}
      </p>
      <div class="row mcp-example-head">
        <p class="label mcp-example-label">
          {{ t("settings.integration.mcpExampleJson") }}
        </p>
        <button
          class="btn"
          type="button"
          @click="copyMcpExample('json')"
        >
          {{
            mcpJsonCopied
              ? t("settings.integration.copied")
              : t("settings.integration.copy")
          }}
        </button>
      </div>
      <pre class="code-area">{{ mcpExampleJson }}</pre>
      <p class="card-desc agent-hint">
        {{ t("settings.integration.mcpTimeoutNote") }}
      </p>
      <div class="row mcp-example-head">
        <p class="label mcp-example-label">
          {{ t("settings.integration.mcpExampleToml") }}
        </p>
        <button
          class="btn"
          type="button"
          @click="copyMcpExample('toml')"
        >
          {{
            mcpTomlCopied
              ? t("settings.integration.copied")
              : t("settings.integration.copy")
          }}
        </button>
      </div>
      <pre class="code-area">{{ mcpExampleToml }}</pre>
    </template>
  </div>

  </div>
  <div class="integration-auto">
  <!-- 自动集成：每个 Agent 一张卡，CLI | MCP | 未集成 三态切换 -->
  <p class="section-title">{{ t("settings.integration.autoTitle") }}</p>

  <div v-if="integrationLoading" class="card settings-section-loading">
    <span class="permission-state-spinner" aria-hidden="true"></span>
    <p>{{ t("common.loading") }}</p>
  </div>

  <template v-else>
  <!-- 待更新总览（跨所有 Agent）：有任意产物过期/缺失时出现，附「全部更新」按钮 -->
  <div v-if="updateSummary.total > 0" class="card update-overview">
    <div class="row">
      <span class="dot-update"></span>
      <div class="overview-text">
        <p class="overview-title">
          {{ t("settings.integration.updatesAvailable") }}
        </p>
        <p class="overview-counts">
          <template v-if="updateSummary.rule > 0"
            >{{ t("settings.integration.rulesLabel") }} ×{{
              updateSummary.rule
            }}</template
          ><template v-if="updateSummary.hook > 0"
            ><span v-if="updateSummary.rule > 0" class="sep">·</span
            >{{ t("settings.integration.hookLabel") }} ×{{
              updateSummary.hook
            }}</template
          ><template v-if="updateSummary.mcp > 0"
            ><span
              v-if="updateSummary.rule > 0 || updateSummary.hook > 0"
              class="sep"
              >·</span
            >{{ t("settings.integration.mcpConfigLabel") }} ×{{
              updateSummary.mcp
            }}</template
          >
        </p>
      </div>
      <span class="spacer"></span>
      <button
        class="btn btn-update"
        type="button"
        :disabled="updateAllBusy"
        @click="updateAll"
      >
        {{ t("settings.integration.updateAll") }}
      </button>
    </div>
  </div>

  <div
    v-for="a in AGENTS"
    :id="`integration-${a.id}`"
    :key="a.id"
    class="card agent-card"
    :class="{ 'settings-target-highlight': settingsTargetHighlight === `integration-${a.id}` }"
    @settings-reveal="openAgentDetails(a.id)"
  >
    <div class="agent-head">
      <BrandBadge :id="a.id" :label="a.title" />
      <div class="agent-id">
        <div class="agent-name-row">
          <p class="card-title">{{ a.title }}</p>
          <span
            v-for="tag in tagsOf(a.id)"
            :key="tag.key"
            class="status-tag"
            :class="tag.tone"
            :data-testid="`agent-tag-${a.id}-${tag.key}`"
          >
            {{ t(`settings.integration.tag.${tag.key}`) }}
          </span>
        </div>
        <p v-if="config && detailsOf(a.id) && !agentExpanded[a.id]" class="card-desc collab-summary">
          {{ t("settings.integration.collabSummary", { style: collaborationSummary(a.id) }) }}
        </p>
      </div>
      <div v-if="!modeReadError?.[a.id]" class="segmented">
        <button
          v-if="a.hasCli"
          type="button"
          class="seg"
          :class="{ active: modes[a.id].mode === 'cli' }"
          :disabled="modeBusy[a.id]"
          @click="setMode(a.id, 'cli')"
        >
          {{ t("settings.integration.modeCli")
          }}<span v-if="a.recommended === 'cli'" class="seg-rec">{{
            t("settings.integration.recommendedTag")
          }}</span>
        </button>
        <button
          v-if="a.hasMcp"
          type="button"
          class="seg"
          :class="{ active: modes[a.id].mode === 'mcp' }"
          :disabled="modeBusy[a.id]"
          @click="setMode(a.id, 'mcp')"
        >
          {{ t("settings.integration.modeMcp")
          }}<span v-if="a.recommended === 'mcp'" class="seg-rec">{{
            t("settings.integration.recommendedTag")
          }}</span>
        </button>
        <button
          type="button"
          class="seg"
          :class="{ active: modes[a.id].mode === 'none' }"
          :disabled="modeBusy[a.id]"
          @click="setMode(a.id, 'none')"
        >
          {{ t("settings.integration.modeNone") }}
        </button>
      </div>
    </div>

    <!-- A full-width bar, not a small link: it is the only hint that an integrated card holds more settings. -->
    <button
      v-if="detailsOf(a.id)"
      type="button"
      class="agent-disclosure"
      :aria-expanded="agentExpanded[a.id]"
      :aria-controls="`agent-details-${a.id}`"
      :data-testid="`agent-details-toggle-${a.id}`"
      @click="toggleAgentDetails(a.id)"
    >
      <span class="agent-disclosure-label">{{ t("settings.integration.details") }}</span>
      <span class="agent-disclosure-hint">{{ t("settings.integration.detailsHint") }}</span>
      <svg
        class="agent-disclosure-chevron"
        :class="{ open: agentExpanded[a.id] }"
        viewBox="0 0 16 16"
        aria-hidden="true"
      >
        <path d="M4 6l4 4 4-4" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </button>

    <p v-if="a.id === 'antigravity'" class="card-desc">{{ t("settings.integration.antigravityScope") }}</p>
    <button v-if="modeReadError?.[a.id]" type="button" class="btn" @click="refreshMode(a.id)">{{ t("settings.integration.retryStatus") }}</button>

    <div
      v-if="!modeReadError?.[a.id] && modes[a.id].mode === 'none' && modes[a.id].lifecycle.cleanupRequired"
      :id="`lifecycle-${a.id}`"
      class="result err lifecycle-cleanup row"
      :class="{ 'settings-target-highlight': settingsTargetHighlight === `lifecycle-${a.id}` }"
    >
      <span>{{ t("settings.integration.lifecycleCleanupHint") }}</span>
      <span class="spacer"></span>
      <button
        class="btn btn-update"
        type="button"
        :disabled="modeBusy[a.id]"
        @click="updateArtifact(a.id, 'hook')"
      >
        {{ t("settings.integration.update") }}
      </button>
    </div>

    <p
      v-if="messageUnderHeader(a.id)"
      class="result"
      :class="modeError[a.id] ? 'err' : 'ok'"
    >
      {{ modeMessage[a.id] }}
    </p>

    <!-- Details: every row of the integrated card. Folded with v-show, not v-if, so a deep link or a
         settings-search hit can still find its row in the DOM and then open the card. -->
    <div
      v-if="detailsOf(a.id) || config"
      v-show="detailsOf(a.id) && agentExpanded[a.id]"
      :id="`agent-details-${a.id}`"
      class="agent-details"
    >
      <template v-if="detailsOf(a.id)">
      <hr class="divider" />

      <p
        v-if="a.id === 'pi' && piVersionLoading"
        class="card-desc settings-inline-loading"
      >
        <span class="permission-state-spinner" aria-hidden="true"></span>
        {{ t("common.loading") }}
      </p>
      <p
        v-else-if="a.id === 'pi' && !modes[a.id].versionSupported"
        class="result err"
      >
        {{
          modes[a.id].agentVersion
            ? t("settings.integration.piVersionUnsupported", {
                version: modes[a.id].agentVersion,
                minimum: modes[a.id].minimumVersion,
              })
            : t("settings.integration.piVersionMissing", {
                minimum: modes[a.id].minimumVersion,
              })
        }}
      </p>

      <p
        v-if="a.id === 'codex' && !modes[a.id].versionSupported"
        class="result err"
      >
        {{
          t("settings.integration.codexVersionUnsupported", {
            version: modes[a.id].agentVersion ?? "",
            minimum: modes[a.id].minimumVersion ?? "",
          })
        }}
      </p>

      <!-- Rules / Skill / SessionStart hook（CLI / MCP 共有；Grok 为 skill，Codex 为协议 hook） -->
      <div class="row agent-row">
        <span class="label">{{
          a.instructionKind === "skill"
            ? t("settings.integration.skillLabel")
            : a.instructionKind === "hook"
              ? t("settings.integration.protocolLabel")
              : t("settings.integration.rulesLabel")
        }}</span>
        <span class="badge">
          <span
            class="dot"
            :class="modes[a.id].ruleInstalled ? 'on' : 'off'"
          ></span>
          {{
            modes[a.id].ruleInstalled
              ? t("settings.integration.installed")
              : t("settings.integration.notInstalled")
          }}
        </span>
        <span class="spacer"></span>
        <button
          v-if="modes[a.id].ruleNeedsUpdate"
          class="btn btn-update"
          type="button"
          :disabled="modeBusy[a.id]"
          @click="updateArtifact(a.id, 'rule')"
        >
          <span class="dot-update"></span
          >{{ t("settings.integration.update") }}
        </button>
        <div v-if="modes[a.id].ruleInstalled" class="menu-wrap">
          <button
            class="btn"
            type="button"
            @click.stop="toggleOpenMenu(a.id + ':rule')"
          >
            {{ t("settings.integration.openFile") }}
          </button>
          <div v-if="openMenuKey === a.id + ':rule'" class="menu-pop">
            <button
              class="menu-item"
              type="button"
              @click="revealFile(a.id, 'rule')"
            >
              {{ revealLabel }}
            </button>
            <button
              class="menu-item"
              type="button"
              @click="openFile(a.id, 'rule')"
            >
              {{ t("settings.integration.openFileAction") }}
            </button>
          </div>
        </div>
      </div>
      <p v-if="modes[a.id].rulePath" class="agent-path">
        {{ modes[a.id].rulePath }}
      </p>
      <p v-if="a.id === 'cursor'" class="card-desc agent-hint">
        {{ t("settings.integration.cursorRulesHint") }}
      </p>
      <p v-if="a.id === 'grok'" class="card-desc agent-hint">
        {{ t("settings.integration.grokSkillHint") }}
      </p>
      <p v-if="a.id === 'codex'" class="card-desc agent-hint">
        {{ t("settings.integration.codexProtocolHint") }}
      </p>

      <!-- Delegated (non-interactive) runs: a pure preference, default off = silent -->
      <template v-if="modes[a.id].delegated.supported">
        <hr class="divider" />
        <div class="row agent-row">
          <span class="label">{{ t("settings.integration.delegatedTitle") }}</span>
          <span class="spacer"></span>
          <label class="switch">
            <input
              type="checkbox"
              :checked="modes[a.id].delegated.enabled"
              :disabled="modeBusy[a.id]"
              @change="
                toggleDelegated(
                  a.id,
                  ($event.target as HTMLInputElement).checked
                )
              "
            />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc agent-hint">
          {{ t("settings.integration.delegatedHint") }}
        </p>
      </template>

      <!-- CLI 模式：超时 Hook（Codex 无 Hook 给提示） -->
      <template v-if="modes[a.id].mode === 'cli'">
        <hr class="divider" />
        <template v-if="a.hasTimeoutHook">
          <div class="row agent-row">
            <span class="label">{{
              modes[a.id].runtimeArtifactKind === "extension"
                ? t("settings.integration.extensionLabel")
                : t("settings.integration.hookLabel")
            }}</span>
            <span class="badge">
              <span
                class="dot"
                :class="modes[a.id].timeoutHookInstalled ? 'on' : 'off'"
              ></span>
              {{
                modes[a.id].timeoutHookInstalled
                  ? t("settings.integration.installed")
                  : t("settings.integration.notInstalled")
              }}
            </span>
            <span class="spacer"></span>
            <button
              v-if="modes[a.id].hookNeedsUpdate && !modes[a.id].lifecycle.needsUpdate"
              class="btn btn-update"
              type="button"
              :disabled="modeBusy[a.id]"
              @click="updateArtifact(a.id, 'hook')"
            >
              <span class="dot-update"></span
              >{{ t("settings.integration.update") }}
            </button>
            <div v-if="modes[a.id].timeoutHookInstalled" class="menu-wrap">
              <button
                class="btn"
                type="button"
                @click.stop="toggleOpenMenu(a.id + ':hook')"
              >
                {{ t("settings.integration.openFile") }}
              </button>
              <div v-if="openMenuKey === a.id + ':hook'" class="menu-pop">
                <button
                  class="menu-item"
                  type="button"
                  @click="revealFile(a.id, 'hook')"
                >
                  {{ revealLabel }}
                </button>
                <button
                  class="menu-item"
                  type="button"
                  @click="openFile(a.id, 'hook')"
                >
                  {{ t("settings.integration.openFileAction") }}
                </button>
              </div>
            </div>
          </div>
          <p class="card-desc agent-hint">
            {{
              modes[a.id].runtimeArtifactKind === "extension"
                ? t("settings.integration.piExtensionHint")
                : t("settings.integration.hookShort")
            }}
          </p>
          <p
            v-if="!modes[a.id].timeoutHookSupported"
            class="result err"
          >
            {{ t("settings.integration.hookUnsupported") }}
          </p>
        </template>
        <template v-else-if="a.id === 'antigravity'">
          <div class="row agent-row">
            <span class="label">{{
              t("settings.integration.contextRecoveryHookLabel")
            }}</span>
            <span class="badge">
              <span
                class="dot"
                :class="modes[a.id].recoveryHookInstalled ? 'on' : 'off'"
              ></span>
              {{
                t("settings.integration.capabilityUnsupported")
              }}
            </span>
            <span class="spacer"></span>
          </div>
          <p class="card-desc agent-hint">
            {{ t("settings.integration.antigravityRecoveryUnsupported") }}
          </p>
        </template>
        <!-- Codex has no timeout hook; the protocol row above already is its SessionStart hook. -->
        <p v-else class="card-desc agent-hint">
          {{ t("settings.integration.codexCliHint") }}
        </p>
      </template>

      <!-- MCP 模式：MCP 配置 -->
      <template v-if="modes[a.id].mode === 'mcp' && modes[a.id].mcpSupported">
        <hr class="divider" />
        <div class="row agent-row">
          <span class="label">{{
            t("settings.integration.mcpConfigLabel")
          }}</span>
          <span class="badge">
            <span
              class="dot"
              :class="modes[a.id].mcpConfigInstalled ? 'on' : 'off'"
            ></span>
            {{
              modes[a.id].mcpConfigInstalled
                ? t("settings.integration.installed")
                : t("settings.integration.notInstalled")
            }}
          </span>
          <span class="spacer"></span>
          <button
            v-if="modes[a.id].mcpNeedsUpdate"
            class="btn btn-update"
            type="button"
            :disabled="modeBusy[a.id]"
            @click="updateArtifact(a.id, 'mcp')"
          >
            <span class="dot-update"></span
            >{{ t("settings.integration.update") }}
          </button>
          <div v-if="modes[a.id].mcpConfigInstalled" class="menu-wrap">
            <button
              class="btn"
              type="button"
              @click.stop="toggleOpenMenu(a.id + ':mcp')"
            >
              {{ t("settings.integration.openFile") }}
            </button>
            <div v-if="openMenuKey === a.id + ':mcp'" class="menu-pop">
              <button
                class="menu-item"
                type="button"
                @click="revealFile(a.id, 'mcp')"
              >
                {{ revealLabel }}
              </button>
              <button
                class="menu-item"
                type="button"
                @click="openFile(a.id, 'mcp')"
              >
                {{ t("settings.integration.openFileAction") }}
              </button>
            </div>
          </div>
        </div>
        <p v-if="modes[a.id].mcpConfigPath" class="agent-path">
          {{ modes[a.id].mcpConfigPath }}
        </p>
        <p class="card-desc agent-hint">
          {{ t("settings.integration.mcpModeHint") }}
        </p>
      </template>

      <hr class="divider" />
      <template v-if="modes[a.id].stop.supported">
        <div class="row agent-row">
          <span class="label">{{ t("settings.integration.stopTitle") }}</span>
          <span class="badge">
            <span
              class="dot"
              :class="modes[a.id].stop.installed ? 'on' : 'off'"
            ></span>
            {{
              modes[a.id].stop.installed
                ? t("settings.integration.configured")
                : t("settings.integration.notConfigured")
            }}
          </span>
          <span class="spacer"></span>
          <button
            v-if="modes[a.id].stop.outdated"
            class="btn btn-update"
            type="button"
            :disabled="modeBusy[a.id]"
            @click="toggleStop(a.id, true)"
          >
            <span class="dot-update"></span
            >{{ t("settings.integration.update") }}
          </button>
          <label class="switch">
            <input
              type="checkbox"
              :checked="modes[a.id].stop.enabled"
              :disabled="modeBusy[a.id]"
              @change="
                toggleStop(
                  a.id,
                  ($event.target as HTMLInputElement).checked
                )
              "
            />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc agent-hint">
          {{ t(a.id === 'grok' ? "settings.integration.stopGrokHint" : "settings.integration.stopHint") }}
        </p>
        <p
          v-if="modes[a.id].stop.otherHandlersDetected"
          class="result err"
        >
          {{ t("settings.integration.stopCoexist") }}
        </p>
      </template>
      <p v-else class="card-desc agent-hint">
        {{ t("settings.integration.stopUnsupported") }}
      </p>

      <!-- 接管 Claude 内置的 AskUserQuestion（仅 Claude Code 有这个工具） -->
      <template v-if="modes[a.id].askQuestion.supported">
        <hr class="divider" />
        <div class="row agent-row">
          <span class="label">{{
            t("settings.integration.askQuestionTitle")
          }}</span>
          <span class="badge">
            <span
              class="dot"
              :class="modes[a.id].askQuestion.installed ? 'on' : 'off'"
            ></span>
            {{
              modes[a.id].askQuestion.installed
                ? t("settings.integration.configured")
                : t("settings.integration.notConfigured")
            }}
          </span>
          <span class="spacer"></span>
          <button
            v-if="modes[a.id].askQuestion.outdated"
            class="btn btn-update"
            type="button"
            :disabled="modeBusy[a.id]"
            @click="toggleAskQuestion(a.id, true)"
          >
            <span class="dot-update"></span
            >{{ t("settings.integration.update") }}
          </button>
          <label class="switch">
            <input
              type="checkbox"
              :checked="modes[a.id].askQuestion.enabled"
              :disabled="modeBusy[a.id]"
              @change="
                toggleAskQuestion(
                  a.id,
                  ($event.target as HTMLInputElement).checked
                )
              "
            />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc agent-hint">
          {{ t("settings.integration.askQuestionHint") }}
        </p>
      </template>

      <hr class="divider" />
      <template v-if="modes[a.id].permission.supported">
        <div class="row agent-row">
          <span class="label">{{
            t("settings.integration.permissionTitle")
          }}</span>
          <span class="badge">
            <span
              class="dot"
              :class="modes[a.id].permission.configured ? 'on' : 'off'"
            ></span>
            {{
              modes[a.id].permission.configured
                ? t("settings.integration.configured")
                : t("settings.integration.notConfigured")
            }}
          </span>
          <span class="spacer"></span>
          <button
            v-if="modes[a.id].permissionNeedsUpdate"
            class="btn btn-update"
            type="button"
            :disabled="modeBusy[a.id]"
            @click="updateArtifact(a.id, 'hook')"
          >
            <span class="dot-update"></span
            >{{ t("settings.integration.update") }}
          </button>
          <label class="switch">
            <input
              type="checkbox"
              :checked="modes[a.id].permission.enabled"
              :disabled="modeBusy[a.id]"
              @change="
                togglePermission(
                  a.id,
                  ($event.target as HTMLInputElement).checked
                )
              "
            />
            <span class="track"></span>
          </label>
        </div>
        <p class="card-desc agent-hint">
          {{
            a.id === 'claude'
              ? t("settings.integration.permissionClaudeHint")
              : t("settings.integration.permissionCodexHint")
          }}
        </p>
        <p class="card-desc agent-hint">
          {{ t("settings.integration.permissionInflightHint") }}
        </p>
        <p
          v-if="modes[a.id].permission.knownBlockedReason"
          class="result err"
        >
          {{
            permissionBlockedText(
              modes[a.id].permission.knownBlockedReason as string
            )
          }}
        </p>
        <p
          v-if="modes[a.id].permission.otherHandlersDetected"
          class="result err"
        >
          {{
            a.id === 'claude'
              ? t("settings.integration.permissionClaudeCoexist")
              : t("settings.integration.permissionCodexCoexist")
          }}
        </p>
      </template>
      <p v-else class="card-desc agent-hint">
        {{
          a.id === 'pi'
            ? t("settings.integration.piPermissionUnsupported")
            : t("settings.integration.permissionUnsupported")
        }}
      </p>

      <hr class="divider" />
      <div
        :id="`lifecycle-${a.id}`"
        class="row agent-row readiness-target-row"
        :class="{ 'settings-target-highlight': settingsTargetHighlight === `lifecycle-${a.id}` }"
      >
        <span class="label">{{ t("settings.integration.lifecycleTitle") }}</span>
        <span class="badge">
          <span
            class="dot"
            :class="modes[a.id].lifecycle.installed ? 'on' : 'off'"
          ></span>
          {{
            !modes[a.id].lifecycle.supported
              ? t("settings.integration.capabilityUnsupported")
              : modes[a.id].lifecycle.installed
              ? t("settings.integration.configured")
              : t("settings.integration.notConfigured")
          }}
        </span>
        <span class="spacer"></span>
        <button
          v-if="modes[a.id].lifecycle.needsUpdate"
          class="btn btn-update"
          type="button"
          :disabled="modeBusy[a.id]"
          @click="updateArtifact(a.id, 'hook')"
        >
          <span class="dot-update"></span
          >{{ t("settings.integration.update") }}
        </button>
        <label v-if="modes[a.id].lifecycle.supported" class="switch">
          <input
            type="checkbox"
            :checked="modes[a.id].lifecycle.enabled"
            :disabled="modeBusy[a.id] || !modes[a.id].lifecycle.supported"
            @change="
              toggleLifecycle(
                a.id,
                ($event.target as HTMLInputElement).checked
              )
            "
          />
          <span class="track"></span>
        </label>
      </div>
      <p class="card-desc agent-hint">
        {{ t(a.id === 'antigravity' ? "settings.integration.antigravityLifecycleUnsupported" : "settings.integration.lifecycleHint") }}
      </p>

      <CodexDesktopSettings v-if="isMac && a.id === 'codex'" />
      <CursorDesktopSettings v-if="a.id === 'cursor'" />

      <p
        v-if="modeMessage[a.id] && agentExpanded[a.id]"
        class="result"
        :class="modeError[a.id] ? 'err' : 'ok'"
      >
        {{ modeMessage[a.id] }}
      </p>
      </template>
      <hr v-if="config" class="divider" />
      <CollaborationStyleEditor
        v-if="config" :general="config.general" :agent="a.id" :defaults="collabDefaults"
        :busy="collabBusy[a.id]" :error="collabError[a.id]"
        :message="collabMessage[a.id]" :results="collabResults[a.id]"
        @save="(style, text) => saveAgentCollaborationStyle(a.id, style, text)"
        @retry="retryCollaborationStyle"
      />
    </div>
  </div>
  </template>
  </div>
</template>

<style scoped>
.prompt-agent-choice { width: auto; max-width: 150px; }
.collab-summary { margin: 4px 0 0; }
</style>
