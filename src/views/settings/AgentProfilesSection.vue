<script setup lang="ts">
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type {
  AgentId,
  DiscoveredProfileInfo,
  ExtraProfileStatus,
} from "../../lib/types";

const props = defineProps<{
  agent: AgentId;
  profiles: ExtraProfileStatus[];
  discovered: DiscoveredProfileInfo[];
  busy: boolean;
}>();

const emit = defineEmits<{
  (e: "add", root: string, label?: string, source?: "discovered" | "manual"): void;
  (e: "toggle", id: string, enabled: boolean, detach?: boolean): void;
  (e: "remove", id: string, detach?: boolean): void;
  (e: "sync", id: string): void;
  (e: "dismiss", root: string): void;
  (e: "reveal", id: string): void;
}>();

const { t } = useI18n();
const adding = ref(false);
const draftRoot = ref("");
const draftLabel = ref("");

async function pickDirectory() {
  try {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string" && selected.trim()) {
      draftRoot.value = selected.trim();
    }
  } catch {
    /* dialog unavailable in non-tauri test environments */
  }
}

function submitAdd() {
  const root = draftRoot.value.trim();
  if (!root || props.busy) return;
  const label = draftLabel.value.trim() || undefined;
  emit("add", root, label, "manual");
  draftRoot.value = "";
  draftLabel.value = "";
  adding.value = false;
}

function dotClass(state: ExtraProfileStatus["state"]): string {
  if (state === "synced") return "on";
  if (state === "conflict" || state === "rootMissing") return "err";
  return "off";
}

function stateLabel(state: ExtraProfileStatus["state"]): string {
  return t(`settings.integration.profiles.state.${state}`);
}
</script>

<template>
  <hr class="divider" />
  <div class="profiles-section" :data-testid="`agent-profiles-${agent}`">
    <div class="row agent-row">
      <span class="label">{{ t("settings.integration.profiles.title") }}</span>
      <span class="spacer"></span>
      <button
        type="button"
        class="btn"
        :disabled="busy"
        :data-testid="`agent-profile-add-toggle-${agent}`"
        @click="adding = !adding"
      >
        {{ adding ? t("common.cancel") : t("settings.integration.profiles.addButton") }}
      </button>
    </div>
    <p class="card-desc agent-hint">
      {{ t(agent === "antigravity" ? "settings.integration.profiles.antigravityHint" : "settings.integration.profiles.hint") }}
    </p>

    <!-- Discovered isolated Antigravity directories awaiting explicit opt-in -->
    <div
      v-for="item in discovered"
      :key="item.root"
      class="profile-item profile-discovered"
      :data-testid="`discovered-profile-${item.root}`"
    >
      <div class="row profile-item-head">
        <span class="profile-name">{{ item.label }}</span>
        <span class="badge">{{ t("settings.integration.profiles.discoveredBadge") }}</span>
        <span class="spacer"></span>
        <button
          type="button"
          class="btn btn-update"
          :disabled="busy"
          @click="emit('add', item.root, item.label, 'discovered')"
        >
          {{ t("settings.integration.profiles.enableAction") }}
        </button>
        <button
          type="button"
          class="btn"
          :disabled="busy"
          @click="emit('dismiss', item.root)"
        >
          {{ t("settings.integration.profiles.ignoreAction") }}
        </button>
      </div>
      <p class="agent-path" :title="item.root">{{ item.displayRoot || item.root }}</p>
    </div>

    <!-- Configured extra profiles -->
    <div
      v-for="profile in profiles"
      :key="profile.id"
      class="profile-item"
      :data-testid="`extra-profile-${profile.id}`"
    >
      <div class="row profile-item-head">
        <span class="profile-name">{{ profile.label }}</span>
        <span class="badge">
          <span class="dot" :class="dotClass(profile.state)"></span>
          {{ stateLabel(profile.state) }}
        </span>
        <span class="spacer"></span>
        <button
          v-if="profile.enabled && profile.needsUpdate"
          type="button"
          class="btn btn-update"
          :disabled="busy"
          @click="emit('sync', profile.id)"
        >
          <span class="dot-update"></span>{{ t("settings.integration.profiles.syncAction") }}
        </button>
        <button
          v-if="profile.state !== 'rootMissing'"
          type="button"
          class="btn"
          :disabled="busy"
          @click="emit('reveal', profile.id)"
        >
          {{ t("settings.integration.profiles.revealAction") }}
        </button>
        <button
          v-if="profile.state === 'conflict'"
          type="button"
          class="btn"
          :disabled="busy"
          @click="emit('remove', profile.id, true)"
        >
          {{ t("settings.integration.profiles.detachAction") }}
        </button>
        <button
          type="button"
          class="btn"
          :disabled="busy"
          @click="emit('remove', profile.id, false)"
        >
          {{ t("settings.integration.profiles.removeAction") }}
        </button>
        <label class="switch">
          <input
            type="checkbox"
            :checked="profile.enabled"
            :disabled="busy"
            @change="emit('toggle', profile.id, ($event.target as HTMLInputElement).checked, false)"
          />
          <span class="track"></span>
        </label>
      </div>
      <p class="agent-path" :title="profile.root">{{ profile.displayRoot || profile.root }}</p>
      <p v-if="profile.error" class="result err profile-note">{{ profile.error }}</p>
      <p v-if="profile.hostPluginDisabled" class="result err profile-note">
        {{ t("settings.integration.antigravityPluginDisabled") }}
      </p>
      <p
        v-else-if="profile.enabled && profile.mcpGrant === false"
        class="card-desc agent-hint profile-note"
      >
        {{ t("settings.integration.antigravityGrantHint") }}
      </p>
    </div>

    <!-- Add extra directory form -->
    <div v-if="adding" class="profile-add-form" :data-testid="`profile-add-form-${agent}`">
      <div class="row profile-add-row">
        <input
          v-model="draftRoot"
          type="text"
          class="input profile-root-input"
          :placeholder="t('settings.integration.profiles.pathPlaceholder')"
          @keydown.enter.prevent="submitAdd"
        />
        <button type="button" class="btn" :disabled="busy" @click="pickDirectory">
          {{ t("settings.integration.profiles.browseAction") }}
        </button>
      </div>
      <div class="row profile-add-row">
        <input
          v-model="draftLabel"
          type="text"
          class="input profile-label-input"
          :placeholder="t('settings.integration.profiles.labelPlaceholder')"
          @keydown.enter.prevent="submitAdd"
        />
        <button
          type="button"
          class="btn btn-update"
          :disabled="busy || !draftRoot.trim()"
          @click="submitAdd"
        >
          {{ t("settings.integration.profiles.confirmAddAction") }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.profiles-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.profile-item {
  border: 1px solid var(--border, #e5e7eb);
  border-radius: 8px;
  padding: 8px 10px;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.profile-discovered {
  border-style: dashed;
}
.profile-item-head {
  gap: 6px;
  flex-wrap: wrap;
}
.profile-name {
  font-size: 13px;
  font-weight: 500;
}
.profile-note {
  margin: 2px 0 0;
}
.profile-add-form {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding-top: 4px;
}
.profile-add-row {
  gap: 6px;
}
.profile-root-input,
.profile-label-input {
  flex: 1;
}
.dot.err {
  background: #e5484d;
}
</style>
