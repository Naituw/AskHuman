<script setup lang="ts">
// The single place the todos window shows iCloud sync state (spec todo-sync §9.7): one fixed-height
// line at the bottom of the sidebar. Only trouble is coloured; the line never changes the layout
// above it, whatever the status says.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { ageOf, type SyncAction, type SyncLine } from "./syncState";

const props = defineProps<{ line: SyncLine }>();
const emit = defineEmits<{ action: [action: SyncAction] }>();

const { t } = useI18n();

function ageText(ageMs: number): string {
  const age = ageOf(ageMs);
  switch (age.unit) {
    case "justNow":
      return t("todosWin.time.justNow");
    case "seconds":
      return t("todosWin.time.secondsAgo", { n: age.n });
    case "minutes":
      return t("todosWin.time.minutesAgo", { n: age.n });
    case "hours":
      return t("todosWin.time.hoursAgo", { n: age.n });
    default:
      return t("todosWin.time.daysAgo", { n: age.n });
  }
}

const text = computed(() => {
  const l = props.line;
  switch (l.key) {
    case "synced":
      return t("todosWin.sync.synced", { time: ageText(l.ageMs ?? 0) });
    case "offline":
      return t("todosWin.sync.offline", { n: l.count ?? 0 });
    default:
      return t(`todosWin.sync.${l.key}`);
  }
});

const actionText = computed(() => {
  switch (props.line.action) {
    case "enable":
      return t("todosWin.sync.enable");
    case "repair":
      return t("todosWin.sync.repair");
    case "view":
      return t("todosWin.sync.view");
    case "resume":
      return t("todosWin.sync.retry");
    default:
      return "";
  }
});

/** The full reason on hover (a long message does not fit the strip). */
const title = computed(() => (props.line.detail ? `${text.value}: ${props.line.detail}` : text.value));
</script>

<template>
  <div class="td-sync" :class="line.tone" :title="title" data-testid="sync-footer">
    <span class="td-sync-dot" aria-hidden="true"></span>
    <span class="td-sync-text">{{ text }}</span>
    <button v-if="line.action" type="button" class="td-sync-action" @click="emit('action', line.action)">
      {{ actionText }}
    </button>
  </div>
</template>

<style scoped>
.td-sync {
  display: flex;
  align-items: center;
  gap: 6px;
  height: 18px;
  min-width: 0;
}
.td-sync-dot {
  flex: 0 0 auto;
  width: 7px;
  height: 7px;
  border-radius: 50%;
  background: var(--text-tertiary, #8e8e93);
}
.td-sync.ok .td-sync-dot {
  background: #30d158;
}
.td-sync.busy .td-sync-dot {
  background: var(--accent);
}
.td-sync.warn {
  color: #ff9f0a;
}
.td-sync.warn .td-sync-dot {
  background: #ff9f0a;
}
.td-sync.error {
  color: #ff453a;
}
.td-sync.error .td-sync-dot {
  background: #ff453a;
}
.td-sync-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.td-sync-action {
  flex: 0 0 auto;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--accent);
  font: inherit;
  cursor: pointer;
}
.td-sync-action:hover {
  text-decoration: underline;
}
</style>
