import type { PopupInboxRequest } from "../../lib/types";

export function inboxGroups(requests: PopupInboxRequest[], order: string[]) {
  return order.map(path => ({ path, requests: requests.filter(r => r.project === path)
    .sort((a, b) => a.sequence - b.sequence || a.requestId.localeCompare(b.requestId)) }))
    .filter(group => group.requests.length);
}
/** Continue within the current project, then advance through stable project groups. */
export function nextInboxRequest(ordered: PopupInboxRequest[], active: string, removed: Set<string>): string | null {
  const index = ordered.findIndex(r => r.requestId === active);
  const project = ordered[index]?.project;
  const cycle = [...ordered.slice(index + 1), ...ordered.slice(0, index)];
  const survivors = cycle.filter(r => !removed.has(r.requestId));
  return (survivors.find(r => r.project === project) ?? survivors[0])?.requestId ?? null;
}
export function inboxTitle(request: PopupInboxRequest): string {
  const interaction = request.interaction;
  const text = interaction.type === "confirm" ? interaction.request.title
    : interaction.request.questions[0]?.message || interaction.request.message.text;
  return text.replace(/[#*_`\[\]>]/g, "").replace(/\s+/g, " ").trim();
}
export function inboxKind(request: PopupInboxRequest) {
  return request.kind ?? (request.interaction.type === "confirm" ? "permission" : "ask");
}
