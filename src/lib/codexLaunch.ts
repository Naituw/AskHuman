import type { CodexLaunchStatus } from "./ipc";

type Translate = (key: string, values?: Record<string, string>) => string;

/** Explain the actual route using the backend's availability reasons. */
export function codexLaunchDescription(status: CodexLaunchStatus, t: Translate): string {
  if (status.target) {
    const result = t(status.target === "desktop" ? "desktop.willUseApp" : "desktop.willUseCli");
    const reason = status.preference === "desktop" ? status.desktopReason : status.terminalReason;
    return status.target !== status.preference && reason
      ? t("desktop.withReason", { result, reason: t(`desktop.reasons.${reason}`) })
      : result;
  }
  if (status.desktopReason === "appNotDetected" && status.terminalReason === "cliNotDetected") {
    return t("desktop.noneDetected");
  }
  const reasons = [...new Set([status.desktopReason, status.terminalReason].filter(Boolean))]
    .map(reason => t(`desktop.reasons.${reason}`)).join(t("desktop.reasonSeparator"));
  return reasons ? t("desktop.cannotLaunch", { reasons }) : t("desktop.unavailable");
}
