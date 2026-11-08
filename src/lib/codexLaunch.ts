import type { CodexLaunchStatus } from "./ipc";

type Translate = (key: string, values?: Record<string, string>) => string;

/** Why neither route of a new Codex task can be used. Mirrors the backend's `description`. */
export function codexLaunchReasons(status: CodexLaunchStatus, t: Translate): string[] {
  if (status.desktopReason === "appNotDetected" && status.terminalReason === "cliNotDetected") {
    return [t("desktop.noneDetectedReason")];
  }
  return [...new Set([status.desktopReason, status.terminalReason].filter(Boolean))]
    .map(reason => t(`desktop.reasons.${reason}`));
}

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
  const reasons = codexLaunchReasons(status, t).join(t("desktop.reasonSeparator"));
  return reasons ? t("desktop.cannotLaunch", { reasons }) : t("desktop.unavailable");
}
