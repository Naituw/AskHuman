<script setup lang="ts">
// One channel's detail on the wizard's remote page: the platform-side key points, the required
// fields, the auto-detect, a connection test and "Save and enable". All logic is the settings
// page's (`useChannels` + the shared `persist`), so both places agree on every outcome.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { openSettings } from "../../lib/ipc";
import { channelSection, type ChannelDef } from "./channelDefs";
import ProbeRow from "./ProbeRow.vue";
import { idleProbe } from "./useProbe";
import { useSetupContext } from "./useSetup";

const props = defineProps<{ def: ChannelDef }>();

const { t } = useI18n();
const ctx = useSetupContext();
const {
  secretsPresent,
  SECRET_PLACEHOLDER,
  clearSecret,
  openChannelGuide,
  cancelDetect,
  closeChannel,
  saveChannel,
  savingChannel,
  channelSavedId,
  channelSaveError,
  channelOpsFor,
  canProbeChannel,
  probes,
} = ctx;

// The form only renders once the settings are loaded.
const section = computed(
  () => channelSection(ctx.config.value!, props.def.id) as unknown as Record<string, string>,
);
const ops = computed(() => channelOpsFor(props.def.id));

/** The round trip's state while it belongs to this channel; another channel's check is not shown. */
const probe = computed(() =>
  probes.states.channel.channel === props.def.id ? probes.states.channel : idleProbe(),
);

function secretSaved(field: ChannelDef["fields"][number]): boolean {
  return field.secret ? secretsPresent.value[field.secret.present] === true : false;
}
</script>

<template>
  <div class="settings-body setup-body" :data-testid="`channel-form-${def.id}`">
    <button
      class="link setup-back"
      type="button"
      data-testid="channel-back"
      @click="closeChannel"
    >
      ‹ {{ t("setup.remote.back") }}
    </button>

    <div class="card">
      <div class="row">
        <p class="card-title">{{ t(def.titleKey) }}</p>
        <a
          class="link guide-link"
          href="#"
          data-testid="channel-guide"
          @click.prevent="openChannelGuide(def.id)"
        >
          {{ t("settings.channels.setupGuide") }} ↗
        </a>
      </div>

      <ol class="setup-points" data-testid="channel-points">
        <li v-for="n in def.points" :key="n">
          {{ t(`setup.channels.${def.id}.points.p${n}`) }}
        </li>
      </ol>
      <hr class="divider" />

      <div v-for="field in def.fields" :key="field.key" class="field">
        <label>{{ t(field.labelKey) }}</label>
        <div class="row">
          <input
            v-model="section[field.key]"
            class="input"
            style="flex: 1"
            :type="field.secret ? 'password' : 'text'"
            :placeholder="secretSaved(field) ? SECRET_PLACEHOLDER : ''"
            :data-testid="`field-${def.id}-${field.key}`"
          />
          <button
            v-if="secretSaved(field) && field.secret"
            class="btn"
            type="button"
            :data-testid="`clear-${def.id}-${field.key}`"
            @click="clearSecret(field.secret.clear)"
          >
            {{ t("settings.channels.clearSecret") }}
          </button>
          <template v-if="field.detected && ops.runDetect">
            <button
              class="btn"
              type="button"
              :disabled="ops.detecting?.value"
              :data-testid="`detect-${def.id}`"
              @click="ops.runDetect?.()"
            >
              {{
                ops.detecting?.value
                  ? t("settings.channels.detecting")
                  : t("settings.channels.autoDetect")
              }}
            </button>
            <button
              v-if="ops.detecting?.value"
              class="btn"
              type="button"
              @click="cancelDetect"
            >
              {{ t("settings.channels.detectCancel") }}
            </button>
          </template>
        </div>
      </div>

      <i18n-t
        v-if="def.detectHintKey && ops.detectCode?.value"
        :keypath="def.detectHintKey"
        tag="p"
        class="result ok"
        data-testid="detect-hint"
      >
        <template #code
          ><b>{{ ops.detectCode?.value }}</b></template
        >
      </i18n-t>
      <p
        v-if="ops.message.value"
        class="result"
        :class="ops.error.value ? 'err' : 'ok'"
        data-testid="channel-message"
      >
        {{ ops.message.value }}
      </p>

      <div class="row setup-form-actions">
        <button
          class="btn"
          type="button"
          :disabled="ops.testing.value"
          data-testid="channel-test"
          @click="ops.runTest()"
        >
          {{
            ops.testing.value
              ? t("settings.channels.testing")
              : t("settings.channels.testConnection")
          }}
        </button>
        <span class="spacer"></span>
        <button
          class="btn btn-primary"
          type="button"
          :disabled="savingChannel"
          data-testid="channel-save"
          @click="saveChannel(def.id)"
        >
          {{ t("setup.remote.saveEnable") }}
        </button>
      </div>
      <p v-if="channelSaveError === 'missing'" class="error-text" data-testid="channel-missing">
        {{ t("setup.remote.missing") }}
      </p>
      <p v-else-if="channelSaveError" class="error-text">{{ channelSaveError }}</p>
      <p v-if="channelSavedId === def.id" class="result ok" data-testid="channel-saved">
        {{ t("setup.remote.saved") }}
      </p>
      <template v-if="canProbeChannel(def.id)">
        <hr class="divider" />
        <ProbeRow
          kind="channel"
          :state="probe"
          :params="{ channel: t(def.titleKey) }"
          :with-label="false"
          @start="probes.start('channel', def.id)"
          @cancel="probes.cancel('channel')"
        />
      </template>
      <p class="setup-more">
        <a
          class="link"
          href="#"
          data-testid="channel-more"
          @click.prevent="openSettings(`channel#channel-${def.id}`)"
        >
          {{ t("setup.remote.moreOptions") }} ↗
        </a>
      </p>
    </div>
  </div>
</template>
