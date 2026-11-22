<script setup lang="ts">
// The provider part of step 1: three neutral choices, then the instructions of the chosen service
// with copy chips for the strings that have to be typed at the service (docs/specs/ios-setup-experience.md
// §4). No service is recommended (docs/specs/ios-open-access.md OA3); the facts mirror
// docs/wiki/ios-broker.md and are held by iosBrokers.ts.
import { computed, onBeforeUnmount, ref } from "vue";
import { useI18n } from "vue-i18n";
import { openPath } from "../../lib/ipc";
import IosRich from "./IosRich.vue";
import { injectIosChannel } from "./iosChannelContext";
import { IOS_BROKERS, brokerChips, brokerLinks, brokerPreset } from "./iosBrokers";
import type { RichChip } from "./iosRich";

// `folded`: the broker already works, so the instructions are tucked into a disclosure.
defineProps<{ folded?: boolean }>();

const { t, locale } = useI18n();
const { provider, chooseBroker, ios } = injectIosChannel();

const preset = computed(() => (provider.value ? brokerPreset(provider.value) : null));
const links = computed(() =>
  provider.value ? brokerLinks(provider.value, String(locale.value).startsWith("zh")) : {},
);

// A topic of this account as the steps name it, before and after a user name is typed.
const aclPlaceholder = computed(() => {
  const word = preset.value?.aclWord ?? "username";
  return `ah/<${t(`settings.channels.ios.guide.word.${word}`)}>/#`;
});
const chips = computed<Record<string, RichChip>>(() => {
  if (!provider.value) return {};
  const out: Record<string, RichChip> = {};
  for (const [id, value] of Object.entries(brokerChips(provider.value, ios.value.username))) {
    out[id] = { label: value ?? aclPlaceholder.value, value };
  }
  return out;
});

function message(key: string): string {
  return t(`settings.channels.ios.guide.${provider.value}.${key}`, { acl: aclPlaceholder.value });
}

const copied = ref("");
let timer: ReturnType<typeof setTimeout> | undefined;
async function copy(value: string) {
  try {
    await navigator.clipboard.writeText(value);
  } catch {
    /* clipboard unavailable: the string is on screen and can be selected */
  }
  copied.value = value;
  clearTimeout(timer);
  timer = setTimeout(() => (copied.value = ""), 1500);
}
onBeforeUnmount(() => clearTimeout(timer));

function open(url: string) {
  void openPath(url);
}
</script>

<template>
  <component
    :is="folded ? 'details' : 'div'"
    :class="{ 'ios-more': folded, 'ios-guide-fold': folded }"
    data-testid="ios-provider-guide"
  >
    <summary v-if="folded">
      {{
        provider
          ? t("settings.channels.ios.guide.foldNamed", {
              name: t(`settings.channels.ios.tile.${provider}.name`),
            })
          : t("settings.channels.ios.guide.fold")
      }}
    </summary>
    <p class="ios-lead">{{ t("settings.channels.ios.guide.lead") }}</p>
    <div class="ios-tiles" role="radiogroup" :aria-label="t('settings.channels.ios.brokerLabel')">
      <button
        v-for="broker in IOS_BROKERS"
        :key="broker.id"
        type="button"
        class="ios-tile"
        :class="{ active: provider === broker.id }"
        role="radio"
        :aria-checked="provider === broker.id"
        :data-testid="`ios-broker-${broker.id}`"
        @click="chooseBroker(broker.id)"
      >
        <b>{{ t(`settings.channels.ios.tile.${broker.id}.name`) }}</b>
        <span>{{ t(`settings.channels.ios.tile.${broker.id}.sub`) }}</span>
      </button>
    </div>

    <template v-if="preset">
      <!-- Before anyone registers: what to watch for at this service. -->
      <p
        v-if="preset.notice"
        class="ios-notice"
        :class="{ info: preset.notice === 'info' }"
        :data-testid="`ios-notice-${preset.id}`"
      >
        {{ message("notice") }}
      </p>
      <component :is="preset.numbered ? 'ol' : 'ul'" class="ios-list" data-testid="ios-guide-steps">
        <li v-for="step in preset.steps" :key="step.key">
          <IosRich :text="message(step.key)" :links="links" :chips="chips" :copied="copied" @open="open" @copy="copy" />
          <ul v-if="step.sub">
            <li v-for="sub in step.sub" :key="sub">
              <IosRich :text="message(sub)" :links="links" :chips="chips" :copied="copied" @open="open" @copy="copy" />
            </li>
          </ul>
        </li>
      </component>
      <p v-if="preset.footer" class="ios-lead">
        <IosRich :text="message('links')" :links="links" @open="open" />
      </p>
    </template>
  </component>
</template>
