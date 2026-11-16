import type { AntigravityLaunchStatus } from "./ipc";

type Translate = (key: string, values?: Record<string, string>) => string;

/**
 * Why neither route of a new Antigravity task can be used, the preferred one first. Mirrors the
 * backend's `description`: the preference only decides which route goes first, so a dead end is
 * explained for both.
 */
export function antigravityLaunchReasons(status: AntigravityLaunchStatus, t: Translate): string[] {
  const obstacles = status.preference === "desktop"
    ? [status.desktopReason, status.terminalReason]
    : [status.terminalReason, status.desktopReason];
  return [...new Set(obstacles.filter(Boolean))].map(reason => t(`antigravityDesktop.launch.reasons.${reason}`));
}

/** Explain the actual route for a new Antigravity task using the backend's availability reasons. */
export function antigravityLaunchDescription(status: AntigravityLaunchStatus, t: Translate): string {
  if (status.target) {
    const result = t(status.target === "desktop" ? "antigravityDesktop.launch.willUseApp" : "antigravityDesktop.launch.willUseCli");
    // A fallback names why the preferred route was passed over.
    const reason = status.preference === "desktop" ? status.desktopReason : status.terminalReason;
    return status.target !== status.preference && reason
      ? t("desktop.withReason", { result, reason: t(`antigravityDesktop.launch.reasons.${reason}`) })
      : result;
  }
  const reasons = antigravityLaunchReasons(status, t).join(t("desktop.reasonSeparator"));
  return reasons ? t("desktop.cannotLaunch", { reasons }) : t("antigravityDesktop.launch.unavailable");
}
