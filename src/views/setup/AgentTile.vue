<script setup lang="ts">
// One Agent on the wizard's first page, as a selectable tile: logo badge, name, status tags, a
// tick, the way to connect it, and (after applying) how that went. The files applying would touch
// and the per-Agent hints live below the grid (AgentsPage), so every tile stays the same size.
// A `compact` tile is the quiet one-line form for an Agent that was not found here: it shows what
// else AskHuman supports, and unfolds its way-to-connect row only once someone ticks it.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import type { SetupAgentMode, SetupAgentRow } from "../../lib/types";
import BrandBadge from "./BrandBadge.vue";
import { isBlocked, modesOf, rowTags } from "./agentRows";
import { useSetupContext } from "./useSetup";

const props = defineProps<{ row: SetupAgentRow; compact?: boolean }>();

const { t } = useI18n();
const { picks, applyResults, readOnly, applying } = useSetupContext();

const pick = computed(() => picks[props.row.id]);
const blocked = computed(() => isBlocked(props.row));
const disabled = computed(() => readOnly.value || blocked.value || applying.value);
// The page's section title already says "detected", so that tag would only repeat it on every tile.
const tags = computed(() => rowTags(props.row).filter((tag) => tag.key !== "detected"));
const result = computed(() => applyResults[props.row.id]);
const modes = computed(() => modesOf(props.row));

function modeLabel(mode: SetupAgentMode): string {
  const name = mode === "cli" ? "CLI" : "MCP";
  return mode === props.row.recommended
    ? t("setup.agents.recommendedMode", { mode: name })
    : name;
}

/** Choosing how to connect an Agent also means connecting it, so a loose tile gets ticked too. */
function setMode(mode: SetupAgentMode) {
  const current = pick.value;
  if (!current || disabled.value) return;
  current.mode = mode;
  current.checked = true;
}
</script>

<template>
  <label
    v-if="pick"
    class="agent-tile"
    :class="{ on: pick.checked, blocked, readonly: readOnly, compact }"
    :data-testid="`agent-${row.id}`"
  >
    <input
      v-model="pick.checked"
      class="tile-input"
      type="checkbox"
      :disabled="disabled"
      :data-testid="`agent-check-${row.id}`"
    />

    <span class="tile-head">
      <BrandBadge :id="row.id" :label="row.title" :size="compact ? 'sm' : 'md'" />
      <span class="tile-id">
        <span class="tile-name">{{ row.title }}</span>
        <span v-if="tags.length > 0" class="tile-tags">
          <span
            v-for="tag in tags"
            :key="tag.key"
            class="status-tag"
            :class="tag.tone"
            :data-testid="`agent-tag-${row.id}-${tag.key}`"
          >
            {{ t(`setup.agents.tags.${tag.key}`, tag.params ?? {}) }}
          </span>
        </span>
      </span>
      <span class="tile-tick" aria-hidden="true">
        <svg viewBox="0 0 12 12"><path d="M2.6 6.5l2.2 2.2 4.6-5" /></svg>
      </span>
    </span>

    <span v-if="row.statusError" class="tile-note err" :data-testid="`agent-error-${row.id}`">
      {{ t("setup.agents.unreadableDesc", { error: row.statusError }) }}
    </span>
    <span
      v-else-if="row.delivery === 'unsupportedVersion'"
      class="tile-note err"
      :data-testid="`agent-error-${row.id}`"
    >
      {{ t("setup.agents.unsupportedDesc") }}
    </span>

    <span v-if="!compact || pick.checked" class="tile-mode">
      <span class="tile-caption">{{ t("setup.agents.modeLabel") }}</span>
      <span
        v-if="modes.length > 1"
        class="segmented tile-segmented"
        role="group"
        :data-testid="`agent-mode-${row.id}`"
      >
        <button
          v-for="mode in modes"
          :key="mode"
          type="button"
          :class="{ active: pick.mode === mode }"
          :disabled="disabled"
          :aria-pressed="pick.mode === mode"
          :data-testid="`agent-mode-${row.id}-${mode}`"
          @click="setMode(mode)"
        >
          {{ modeLabel(mode) }}
        </button>
      </span>
      <span
        v-else
        class="segmented tile-segmented fixed"
        :data-testid="`agent-mode-${row.id}`"
      >
        <span class="seg-fixed">{{ modeLabel(modes[0]) }}</span>
      </span>
    </span>

    <span
      v-if="result"
      class="tile-note"
      :class="result.ok ? 'ok' : 'err'"
      :data-testid="`agent-result-${row.id}`"
    >
      <template v-if="result.ok">
        ✓ {{ t("setup.agents.resultOk", { mode: result.modeAfter === "cli" ? "CLI" : "MCP" }) }}
      </template>
      <template v-else>✗ {{ result.error }}</template>
    </span>
  </label>
</template>
