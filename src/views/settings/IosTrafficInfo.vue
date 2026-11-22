<script setup lang="ts">
// This month's message traffic, as far as this Mac knows it (docs/specs/ios-traffic-reduction.md):
// the counters of the paired phones converge into one total, with the per-device breakdown behind a
// disclosure. It only counts and says nothing of the broker's plan.
import { computed } from "vue";
import { useI18n } from "vue-i18n";
import { injectIosChannel } from "./iosChannelContext";
import { aggregateTraffic, brokerEstimate, formatBytes, hasCompleteTrafficHistory, trafficReminder } from "./iosTraffic";

const { t } = useI18n();
const { state } = injectIosChannel();

const traffic = computed(() => {
  const local = state.value?.traffic;
  const records = state.value?.usageRecords ?? [];
  const tr = records.length && local ? aggregateTraffic(records, local.month) : local;
  return tr && tr.msgsIn + tr.msgsOut > 0 ? tr : null;
});
const completeTrafficHistory = computed(() => {
  const records = state.value?.usageRecords ?? [];
  return records.length > 0 && !!traffic.value && hasCompleteTrafficHistory(records, traffic.value.month);
});
const trafficText = computed(() => {
  const tr = traffic.value;
  if (!tr) return "";
  return t(completeTrafficHistory.value ? "settings.channels.ios.trafficAggregate" : "settings.channels.ios.trafficPartial", {
    total: formatBytes(brokerEstimate(tr)),
    count: state.value?.usageRecords?.length ?? 1,
    out: formatBytes(tr.bytesOut),
    in: formatBytes(tr.bytesIn),
  });
});
// Only counts, and says nothing of the broker's plan: close to each GB, or past it (same rule as the phone's Inbox).
const trafficNote = computed(() => {
  const reminder = traffic.value ? trafficReminder(brokerEstimate(traffic.value)) : null;
  if (!reminder) return "";
  return t(reminder.reached ? "settings.channels.ios.trafficOver" : "settings.channels.ios.trafficNear", { gb: reminder.gigabytes });
});
</script>

<template>
  <p v-if="trafficText" class="ios-traffic" data-testid="ios-traffic">{{ trafficText }}</p>
  <details v-if="state?.usageRecords?.length" class="card-desc ios-traffic-details">
    <summary>{{ t("settings.channels.ios.trafficDetails") }}</summary>
    <p v-for="record in state.usageRecords" :key="`${record.deviceKind ?? 'mac'}|${record.macId}`">
      {{ record.deviceKind === 'phone' ? t("settings.channels.ios.trafficMobile") : 'Mac' }} · {{ record.macName || record.macId.slice(0, 8) }} · {{ formatBytes(brokerEstimate(record)) }} ·
      {{ t("settings.channels.ios.trafficUpdated", { time: new Date(record.updatedAt).toLocaleString() }) }}
      <template v-if="state.trafficSyncedAt?.[`${record.broker}|${record.month}|${record.deviceKind ?? 'mac'}|${record.macId}`]">
        · {{ t("settings.channels.ios.trafficSynced", { time: new Date(state.trafficSyncedAt[`${record.broker}|${record.month}|${record.deviceKind ?? 'mac'}|${record.macId}`]!).toLocaleString() }) }}
      </template>
    </p>
    <p v-if="!completeTrafficHistory">{{ t("settings.channels.ios.trafficIncomplete") }}</p>
    <p>{{ t("settings.channels.ios.trafficCoverage") }}</p>
  </details>
  <p v-if="state?.trafficStorageIssue" class="card-desc warn">{{ t("settings.channels.ios.trafficStorageIssue") }}</p>
  <p v-if="trafficNote" class="card-desc ios-traffic-note">{{ trafficNote }}</p>
</template>
