import type { CursorLaunchStatus } from "./ipc";

type Translate = (key: string, values?: Record<string, string>) => string;

/** Explain the actual route for a new Cursor task using the backend's availability reasons. */
export function cursorLaunchDescription(status: CursorLaunchStatus, t: Translate): string {
  if (status.target) {
    const result = t(status.target === "desktop" ? "cursorDesktop.launch.willUseWindow" : "cursorDesktop.launch.willUseCli");
    const reason = status.preference === "desktop" ? status.desktopReason : status.terminalReason;
    return status.target !== status.preference && reason
      ? t("desktop.withReason", { result, reason: t(`cursorDesktop.launch.reasons.${reason}`) })
      : result;
  }
  // The Agents window is no obstacle for a task that was never meant to go there.
  const obstacles = status.preference === "desktop" ? [status.desktopReason, status.terminalReason] : [status.terminalReason];
  const reasons = [...new Set(obstacles.filter(Boolean))]
    .map(reason => t(`cursorDesktop.launch.reasons.${reason}`)).join(t("desktop.reasonSeparator"));
  return reasons ? t("desktop.cannotLaunch", { reasons }) : t("cursorDesktop.launch.unavailable");
}
