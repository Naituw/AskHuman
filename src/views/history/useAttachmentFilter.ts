// State and derived data of the history window's attachment filter (docs/specs/reply-history.md §3.7).
//
// The window feeds it the loaded entries and the entries that already passed the project / session /
// keyword filters; it answers with the entries that also pass the attachment conditions, the suffix choices
// for the panel, and the removable chips. The keyword filter is a separate computed on purpose: changing a
// suffix must not re-run it.

import { computed, ref, toRaw, type Ref } from "vue";
import { useI18n } from "vue-i18n";
import {
  EMPTY_ATTACHMENT_FACET,
  NO_ATTACHMENTS,
  NO_EXTENSION,
  attachmentChipKey,
  attachmentChipsOf,
  attachmentFacet,
  buildAttachmentIndex,
  emptyAttachmentFilter,
  isAttachmentFilterActive,
  matchesAttachmentFilter,
  removeAttachmentChip,
  type AttachmentChip,
  type AttachmentFacts,
  type AttachmentFilter,
} from "../../lib/historyAttachments";
import type { HistoryEntry } from "../../lib/types";

export interface AttachmentChipItem {
  key: string;
  chip: AttachmentChip;
  label: string;
}

export function useAttachmentFilter(deps: {
  /** Every loaded entry; facts are computed for all of them once per load. */
  entries: Readonly<Ref<HistoryEntry[]>>;
  /** The entries that passed the project, session and keyword filters. */
  searched: Readonly<Ref<HistoryEntry[]>>;
}) {
  const { t } = useI18n();
  const filter = ref<AttachmentFilter>(emptyAttachmentFilter());
  const open = ref(false);

  // Entries are only ever replaced wholesale, so the facts read the raw array: going through Vue's deep
  // proxies would track every field of every entry for nothing (about ten times slower).
  const index = computed(() => buildAttachmentIndex(toRaw(deps.entries.value)));
  const active = computed(() => isAttachmentFilterActive(filter.value));

  const filtered = computed(() => {
    if (!active.value) return deps.searched.value;
    const conditions = filter.value;
    const facts = index.value;
    return deps.searched.value.filter((entry) =>
      matchesAttachmentFilter(facts.get(entry.id) ?? NO_ATTACHMENTS, conditions)
    );
  });

  // Only counted while the panel is showing it.
  const facet = computed(() =>
    open.value
      ? attachmentFacet(deps.searched.value, index.value, filter.value.source, filter.value.exts)
      : EMPTY_ATTACHMENT_FACET
  );

  function chipLabel(chip: AttachmentChip): string {
    switch (chip.kind) {
      case "has":
        return t("history.attach.chip.has");
      case "source":
        return t(`history.attach.chip.${chip.source}`);
      case "ext":
        return chip.ext === NO_EXTENSION ? t("history.attach.noExt") : `.${chip.ext}`;
    }
  }

  const chips = computed<AttachmentChipItem[]>(() =>
    attachmentChipsOf(filter.value).map((chip) => ({
      key: attachmentChipKey(chip),
      chip,
      label: chipLabel(chip),
    }))
  );
  /** The conditions in words, for the delete confirmation. */
  const summary = computed(() => chips.value.map((item) => item.label).join(" · "));

  function factsOf(entry: HistoryEntry): AttachmentFacts {
    return index.value.get(entry.id) ?? NO_ATTACHMENTS;
  }

  /** Tooltip for the list badge: how many attachments of each kind the entry carries. */
  function badgeTitle(facts: AttachmentFacts): string {
    const parts: string[] = [];
    if (facts.question) parts.push(t("history.attach.badgeQuestion", { n: facts.question }));
    if (facts.reply) parts.push(t("history.attach.badgeReply", { n: facts.reply }));
    return parts.join(" · ");
  }

  function clear() {
    filter.value = emptyAttachmentFilter();
  }

  function removeChip(chip: AttachmentChip) {
    filter.value = removeAttachmentChip(filter.value, chip);
  }

  /** Back to the initial state: nothing selected, panel closed (a new open request does this). */
  function reset() {
    clear();
    open.value = false;
  }

  return { filter, open, active, filtered, facet, chips, summary, factsOf, badgeTitle, clear, removeChip, reset };
}
