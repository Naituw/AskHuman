<script setup lang="ts">
import { computed, ref, watch } from "vue";
import { useI18n } from "vue-i18n";
import type { AgentId, CollaborationStyle, CollaborationStyleApplyResult, GeneralConfig } from "../../lib/types";

const props = defineProps<{
  general: GeneralConfig;
  agent?: AgentId;
  defaults: { aligned: string; autonomous: string };
  busy?: boolean;
  error?: string | null;
  message?: string;
  results?: CollaborationStyleApplyResult[];
}>();
const emit = defineEmits<{
  save: [style: CollaborationStyle | null, customText: string | null];
  retry: [agent: AgentId];
}>();
const { t } = useI18n();
const style = ref<CollaborationStyle | "inherit">("inherit");
const customText = ref<string | null>(null);
const saved = computed(() => props.agent
  ? props.general.agentCollaborationStyles?.[props.agent] ?? { style: null, customText: null }
  : { style: props.general.collaborationStyle, customText: props.general.collaborationStyleCustomText });

watch(() => JSON.stringify(saved.value), () => {
  style.value = saved.value.style ?? "inherit";
  customText.value = saved.value.customText ?? null;
}, { immediate: true });

const dirty = computed(() => style.value !== (saved.value.style ?? "inherit")
  || customText.value !== (saved.value.customText ?? null));
const effectiveStyle = computed(() => style.value === "inherit" ? props.general.collaborationStyle : style.value);
const styles: CollaborationStyle[] = ["aligned", "autonomous", "custom"];
const label = (value: CollaborationStyle) => t(`settings.integration.collab${value[0].toUpperCase()}${value.slice(1)}`);
const emptyCustom = computed(() => style.value === "custom" && !customText.value?.trim());

function selectStyle(value: CollaborationStyle | "inherit") {
  if (value === "custom" && (customText.value === null || (!props.agent && !customText.value.trim()))) {
    const current = effectiveStyle.value;
    const text = style.value === "inherit" ? props.general.collaborationStyleCustomText : customText.value;
    if (!props.agent) customText.value = props.defaults.aligned;
    else if (current === "custom") customText.value = text?.trim() ? text : props.defaults.aligned;
    else customText.value = props.defaults[current];
  }
  style.value = value;
  if (!props.agent && dirty.value) emit("save", value === "inherit" ? null : value, customText.value);
}
</script>

<template>
  <section :id="agent ? `collaboration-${agent}` : 'collaboration-default'" class="collab-style-editor">
    <div class="row">
      <label :for="agent ? `collaboration-choice-${agent}` : undefined" :class="agent ? 'label' : 'card-title'">
        {{ t(agent ? "settings.integration.collabTitle" : "settings.integration.collabDefaultTitle") }}
      </label>
      <span class="spacer"></span>
      <select
        v-if="agent"
        :id="`collaboration-choice-${agent}`"
        class="input collab-choice"
        :value="style"
        :disabled="busy"
        @change="selectStyle(($event.target as HTMLSelectElement).value as CollaborationStyle | 'inherit')"
      >
        <option value="inherit">{{ t("settings.integration.collabInherit") }}</option>
        <option v-for="option in styles" :key="option" :value="option">{{ label(option) }}</option>
      </select>
      <div v-else class="segmented">
        <button v-for="option in styles" :key="option" type="button" class="seg"
          :class="{ active: style === option }" :disabled="busy" @click="selectStyle(option)">
          {{ label(option) }}
        </button>
      </div>
    </div>
    <p v-if="!agent" class="card-desc">{{ t("settings.integration.collabDefaultHint") }}</p>
    <p v-if="style === 'inherit'" class="card-desc">
      {{ t("settings.integration.collabEffective", { style: label(effectiveStyle) }) }}
    </p>
    <p v-else-if="style === 'aligned'" class="card-desc">{{ t("settings.integration.collabAlignedDesc") }}</p>
    <p v-else-if="style === 'autonomous'" class="card-desc">{{ t("settings.integration.collabAutonomousDesc") }}</p>
    <template v-else>
      <p class="card-desc">{{ t("settings.integration.collabCustomDesc") }}</p>
      <textarea :value="customText ?? ''" class="input collab-custom-input" rows="8" spellcheck="false"
        :aria-label="t('settings.integration.collabCustom')" :disabled="busy"
        @input="customText = ($event.target as HTMLTextAreaElement).value" />
      <p v-if="emptyCustom" class="card-desc">{{ t("settings.integration.collabEmptyHint") }}</p>
    </template>
    <div v-if="agent || style === 'custom'" class="row collab-save-row">
      <span class="spacer"></span>
      <button type="button" class="btn" :disabled="busy || !dirty"
        @click="emit('save', style === 'inherit' ? null : style, customText)">
        {{ t("settings.integration.collabSave") }}
      </button>
    </div>
    <p v-if="error" class="result err">{{ error }}</p>
    <p v-if="message" class="result ok">{{ message }}</p>
    <template v-for="result in results" :key="result.agent">
      <div v-if="result.error" class="result err row">
        <span>{{ t("settings.integration.collabApplyFailed", { agent: result.agent, error: result.error }) }}</span>
        <span class="spacer"></span>
        <button type="button" class="btn" :disabled="busy" @click="emit('retry', result.agent)">
          {{ t("settings.integration.update") }}
        </button>
      </div>
    </template>
  </section>
</template>

<style scoped>
.collab-choice { width: auto; max-width: 65%; }
.collab-custom-input { width: 100%; box-sizing: border-box; resize: vertical; font-family: var(--font-mono, monospace); }
.collab-save-row { margin-top: 8px; }
</style>
