<script setup lang="ts">
// One Agent on the wizard's first page: tick, status tags, mode, the files applying would touch,
// and (after applying) the per-row result.
import { computed, ref } from "vue";
import { useI18n } from "vue-i18n";
import type { SetupAgentMode, SetupAgentRow } from "../../lib/types";
import { isBlocked, isPending, modesOf, rowTags } from "./agentRows";
import { useSetupContext } from "./useSetup";

const props = defineProps<{ row: SetupAgentRow }>();

const { t } = useI18n();
const { picks, applyResults, readOnly, applying } = useSetupContext();

const pick = computed(() => picks[props.row.id]);
const blocked = computed(() => isBlocked(props.row));
const disabled = computed(() => readOnly.value || blocked.value || applying.value);
const tags = computed(() => rowTags(props.row));
const result = computed(() => applyResults[props.row.id]);
const pending = computed(() => isPending(props.row, pick.value));
const modes = computed(() => modesOf(props.row));
const files = computed(() => (pick.value ? props.row.planned[pick.value.mode] : []));

const filesOpen = ref(false);

function modeLabel(mode: SetupAgentMode): string {
  const name = mode === "cli" ? "CLI" : "MCP";
  return mode === props.row.recommended
    ? t("setup.agents.recommendedMode", { mode: name })
    : name;
}
</script>

<template>
  <div
    v-if="pick"
    class="setup-agent"
    :class="{ blocked, off: !pick.checked }"
    :data-testid="`agent-${row.id}`"
  >
    <div class="setup-agent-head">
      <label class="setup-check">
        <input
          v-model="pick.checked"
          type="checkbox"
          :disabled="disabled"
          :data-testid="`agent-check-${row.id}`"
        />
        <span class="setup-agent-title">{{ row.title }}</span>
      </label>
      <span
        v-for="tag in tags"
        :key="tag.key"
        class="setup-tag"
        :class="tag.tone"
        :data-testid="`agent-tag-${row.id}-${tag.key}`"
      >
        {{ t(`setup.agents.tags.${tag.key}`, tag.params ?? {}) }}
      </span>
      <span class="spacer"></span>
      <select
        v-if="modes.length > 1"
        v-model="pick.mode"
        class="select"
        :disabled="disabled || !pick.checked"
        :data-testid="`agent-mode-${row.id}`"
      >
        <option v-for="mode in modes" :key="mode" :value="mode">
          {{ modeLabel(mode) }}
        </option>
      </select>
      <span v-else class="setup-mode-fixed" :data-testid="`agent-mode-${row.id}`">
        {{ modeLabel(modes[0]) }}
      </span>
    </div>

    <p v-if="row.statusError" class="setup-agent-err" :data-testid="`agent-error-${row.id}`">
      {{ t("setup.agents.unreadableDesc", { error: row.statusError }) }}
    </p>
    <p
      v-else-if="row.delivery === 'unsupportedVersion'"
      class="setup-agent-err"
      :data-testid="`agent-error-${row.id}`"
    >
      {{ t("setup.agents.unsupportedDesc") }}
    </p>
    <p
      v-if="row.id === 'cursor' && !blocked"
      class="setup-agent-hint"
    >
      {{ t("settings.integration.cursorRulesHint") }}
    </p>
    <p
      v-if="row.delivery === 'untrusted'"
      class="setup-agent-hint"
      :data-testid="`agent-trust-${row.id}`"
    >
      {{ t("setup.agents.codexTrustHint") }}
    </p>

    <p
      v-if="result"
      class="setup-agent-result"
      :class="result.ok ? 'ok' : 'err'"
      :data-testid="`agent-result-${row.id}`"
    >
      <template v-if="result.ok">
        ✓ {{ t("setup.agents.resultOk", { mode: result.modeAfter === "cli" ? "CLI" : "MCP" }) }}
      </template>
      <template v-else>✗ {{ result.error }}</template>
    </p>

    <template v-if="pending && files.length > 0">
      <button class="link setup-files-toggle" type="button" @click="filesOpen = !filesOpen">
        {{ filesOpen ? "▾" : "▸" }} {{ t("setup.agents.files", { n: files.length }) }}
      </button>
      <ul v-if="filesOpen" class="setup-files" :data-testid="`agent-files-${row.id}`">
        <li v-for="file in files" :key="file.path">
          <span class="setup-file-kind">{{
            file.exists ? t("setup.agents.fileModify") : t("setup.agents.fileCreate")
          }}</span>
          <code class="agent-path">{{ file.path }}</code>
        </li>
      </ul>
    </template>
  </div>
</template>
