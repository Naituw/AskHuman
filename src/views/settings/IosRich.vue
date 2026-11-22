<script setup lang="ts">
// One sentence of the iOS App card's instructions with its inline markup turned into elements
// (bold, code, links, strings that copy on click). See iosRich.ts for the markup.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { parseRich, type RichChip } from "./iosRich";

const props = defineProps<{
  text: string;
  links?: Record<string, string>;
  chips?: Record<string, RichChip>;
  /** The value that was just copied, so its chip can say so for a moment. */
  copied?: string;
}>();

const emit = defineEmits<{
  open: [url: string];
  copy: [value: string];
}>();

const { t } = useI18n();
const pieces = computed(() => parseRich(props.text));

function chip(id: string): RichChip {
  return props.chips?.[id] ?? { label: id, value: null };
}
</script>

<template>
  <template v-for="(piece, i) in pieces" :key="i">
    <b v-if="piece.kind === 'bold'">{{ piece.text }}</b>
    <code v-else-if="piece.kind === 'code'">{{ piece.text }}</code>
    <template v-else-if="piece.kind === 'link'">
      <a
        v-if="links?.[piece.id]"
        class="link"
        href="#"
        :data-link="piece.id"
        @click.prevent="emit('open', links[piece.id]!)"
        >{{ piece.text }}</a
      >
      <template v-else>{{ piece.text }}</template>
    </template>
    <button
      v-else-if="piece.kind === 'chip'"
      type="button"
      class="ios-chip"
      :disabled="!chip(piece.id).value"
      :data-chip="piece.id"
      @click="emit('copy', chip(piece.id).value!)"
    >
      {{ chip(piece.id).label
      }}<span v-if="copied && copied === chip(piece.id).value" class="ios-copied">{{
        t("settings.channels.ios.guide.copied")
      }}</span>
    </button>
    <template v-else>{{ piece.text }}</template>
  </template>
</template>
