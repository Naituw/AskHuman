import type { CursorLaunchStatus } from "./ipc";

type Translate = (key: string, values?: Record<string, string>) => string;

/**
 * Why neither route of a new Cursor task can be used, the preferred one first. Mirrors the
 * backend's `description`: the preference only decides which route goes first, so a dead end is
 * explained for both.
 */
export function cursorLaunchReasons(status: CursorLaunchStatus, t: Translate): string[] {
  const obstacles = status.preference === "desktop"
    ? [status.desktopReason, status.terminalReason]
    : [status.terminalReason, status.desktopReason];
  return [...new Set(obstacles.filter(Boolean))].map(reason => t(`cursorDesktop.launch.reasons.${reason}`));
}

/** Explain the actual route for a new Cursor task using the backend's availability reasons. */
export function cursorLaunchDescription(status: CursorLaunchStatus, t: Translate): string {
  if (status.target) {
    const result = t(status.target === "desktop" ? "cursorDesktop.launch.willUseWindow" : "cursorDesktop.launch.willUseCli");
    // A fallback names why the preferred route was passed over.
    const reason = status.preference === "desktop" ? status.desktopReason : status.terminalReason;
    return status.target !== status.preference && reason
      ? t("desktop.withReason", { result, reason: t(`cursorDesktop.launch.reasons.${reason}`) })
      : result;
  }
  const reasons = cursorLaunchReasons(status, t).join(t("desktop.reasonSeparator"));
  return reasons ? t("desktop.cannotLaunch", { reasons }) : t("cursorDesktop.launch.unavailable");
}
