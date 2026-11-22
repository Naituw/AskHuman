<script setup lang="ts">
// The broker connection (host, ports, user name, password), its test, and the settings that rarely
// change. One form for step 1 and for the "Connection settings" of a finished setup
// (docs/specs/ios-setup-experience.md §4.3). The password is always typed here, by the person.
import { ref } from "vue";
import { useI18n } from "vue-i18n";
import { useSettingsContext } from "./context";
import { injectIosChannel } from "./iosChannelContext";

// `showExport`: the finished setup also offers the connection string for another Mac.
defineProps<{ showExport?: boolean }>();

const { t } = useI18n();
const { persist, secretsPresent, SECRET_PLACEHOLDER, clearSecret } = useSettingsContext();
const {
  ios,
  defaultRelayUrl,
  relayMode,
  setRelayMode,
  testing,
  testMessage,
  testError,
  testWarnings,
  testHint,
  runTest,
  importText,
  connectionMessage,
  connectionError,
  importConnection,
  exportConnection,
} = injectIosChannel();

const importOpen = ref(false);

async function doImport() {
  await importConnection(t("settings.channels.ios.importDone"));
  if (!connectionError.value) importOpen.value = false;
}
</script>

<template>
  <div class="ios-form">
    <div class="field">
      <label>{{ t("settings.channels.ios.host") }}</label>
      <input
        class="input"
        v-model="ios.host"
        spellcheck="false"
        :placeholder="t('settings.channels.ios.hostPlaceholder')"
        data-testid="ios-host"
        @change="persist"
      />
    </div>
    <div class="ios-ports">
      <div class="field">
        <label>{{ t("settings.channels.ios.port") }}</label>
        <input class="input" type="number" v-model.number="ios.port" @change="persist" />
      </div>
      <div class="field">
        <label>{{ t("settings.channels.ios.wsPort") }}</label>
        <input class="input" type="number" v-model.number="ios.wsPort" @change="persist" />
      </div>
    </div>
    <div class="field">
      <label>{{ t("settings.channels.ios.username") }}</label>
      <input class="input" v-model="ios.username" spellcheck="false" data-testid="ios-username" @change="persist" />
    </div>
    <div class="field">
      <label>{{ t("settings.channels.ios.password") }}</label>
      <div class="row">
        <input
          class="input"
          style="flex: 1"
          type="password"
          :placeholder="secretsPresent.iosBrokerPassword ? SECRET_PLACEHOLDER : ''"
          v-model="ios.password"
          data-testid="ios-password"
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

    <div class="row ios-form-actions">
      <button class="btn" type="button" :disabled="testing" data-testid="ios-test" @click="runTest">
        {{ testing ? t("settings.channels.testing") : t("settings.channels.testConnection") }}
      </button>
      <button class="btn" type="button" data-testid="ios-import-open" @click="importOpen = !importOpen">
        {{ t("settings.channels.ios.importOpen") }}
      </button>
      <button
        v-if="showExport"
        class="btn"
        type="button"
        data-testid="ios-export"
        @click="exportConnection(t('settings.channels.ios.exportDone'))"
      >
        {{ t("settings.channels.ios.exportButton") }}
      </button>
      <span class="spacer"></span>
    </div>
    <p v-if="testMessage" class="result" :class="testError ? 'err' : 'ok'" data-testid="ios-test-result">
      {{ testMessage }}
    </p>
    <p v-if="testHint" class="ios-test-hint">
      {{ t(`settings.channels.ios.broker.${testHint.broker}.hint.${testHint.kind}`) }}
    </p>
    <p v-for="warning in testWarnings" :key="warning" class="result warn">⚠ {{ warning }}</p>

    <div v-if="importOpen" class="field ios-import">
      <label>{{ t("settings.channels.ios.importLabel") }}</label>
      <div class="row">
        <input
          class="input"
          style="flex: 1"
          v-model="importText"
          spellcheck="false"
          :placeholder="t('settings.channels.ios.importPlaceholder')"
          data-testid="ios-import-input"
          @keydown.enter.prevent="doImport"
        />
        <button class="btn" type="button" :disabled="!importText.trim()" data-testid="ios-import" @click="doImport">
          {{ t("settings.channels.ios.importButton") }}
        </button>
      </div>
    </div>
    <p v-if="connectionMessage" class="result" :class="connectionError ? 'err' : 'ok'">
      {{ connectionMessage }}
    </p>

    <details class="ios-more">
      <summary>{{ t("settings.channels.ios.moreSettings") }}</summary>
      <div class="field ios-relay">
        <label>{{ t(defaultRelayUrl ? "settings.channels.ios.relayTitle" : "settings.channels.ios.relayUrl") }}</label>
        <!-- Only a build with a public relay offers the choice; otherwise it is just the address. -->
        <select
          v-if="defaultRelayUrl"
          class="select ios-relay-mode"
          :value="relayMode"
          @change="setRelayMode(($event.target as HTMLSelectElement).value === 'own' ? 'own' : 'public')"
        >
          <option value="public">{{ t("settings.channels.ios.relayPublic") }}</option>
          <option value="own">{{ t("settings.channels.ios.relayOwn") }}</option>
        </select>
        <p v-if="defaultRelayUrl && relayMode === 'public'" class="card-desc ios-relay-note">
          {{ t("settings.channels.ios.relayPublicNote") }}
        </p>
        <input
          v-if="!defaultRelayUrl || relayMode === 'own'"
          class="input"
          v-model="ios.relayUrl"
          spellcheck="false"
          :placeholder="defaultRelayUrl ? t('settings.channels.ios.relayOwnPlaceholder') : ''"
          @change="persist"
        />
        <!-- An empty address still means the public relay, so say so while "own" has none yet. -->
        <p v-if="defaultRelayUrl && relayMode === 'own' && !ios.relayUrl.trim()" class="card-desc ios-relay-pending">
          {{ t("settings.channels.ios.relayOwnEmpty") }}
        </p>
      </div>
      <div class="field">
        <label>{{ t("settings.channels.ios.macName") }}</label>
        <input
          class="input"
          v-model="ios.macName"
          :placeholder="t('settings.channels.ios.macNamePlaceholder')"
          @change="persist"
        />
      </div>
    </details>
  </div>
</template>
