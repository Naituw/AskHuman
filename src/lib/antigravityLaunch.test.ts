import { expect, it } from "vitest";
import { antigravityLaunchDescription, antigravityLaunchReasons } from "./antigravityLaunch";
import { i18n } from "../i18n";
import type { AntigravityLaunchStatus } from "./ipc";

const t = (key: string, values?: Record<string, string>) => i18n.global.t(key, values ?? {});

function status(patch: Partial<AntigravityLaunchStatus>): AntigravityLaunchStatus {
  return {
    preference: "desktop",
    target: "desktop",
    desktopAvailable: true,
    terminalAvailable: false,
    desktopReason: null,
    terminalReason: "cliLaunchNotAvailable",
    integrated: true,
    ...patch,
  };
}

it("says the App is used, without a reason, when it is both preferred and available", () => {
  i18n.global.locale.value = "en";
  expect(antigravityLaunchDescription(status({}), t)).toBe("New tasks will use the Antigravity App");
});

it("says why the App was passed over when agy is used instead", () => {
  i18n.global.locale.value = "en";
  expect(
    antigravityLaunchDescription(
      status({ target: "terminal", desktopAvailable: false, terminalAvailable: true, desktopReason: "sidecarsNotAllowed", terminalReason: null }),
      t,
    ),
  ).toBe(
    "New tasks will use agy in a terminal (AskHuman is not allowed to manage Antigravity sidecars yet (turn it on in Settings))",
  );
});

it("names both problems when neither route works, the preferred one leading", () => {
  i18n.global.locale.value = "en";
  const stuck = status({ target: null, desktopAvailable: false, desktopReason: "appNotInstalled", terminalReason: "cliNotDetected" });
  expect(antigravityLaunchDescription(stuck, t)).toBe(
    "Unable to start: The Antigravity App was not detected; The agy command was not detected",
  );
  expect(antigravityLaunchDescription({ ...stuck, preference: "terminal" }, t)).toBe(
    "Unable to start: The agy command was not detected; The Antigravity App was not detected",
  );
});

it("lists each reason once and nothing when both routes work", () => {
  i18n.global.locale.value = "en";
  const same = status({ target: null, desktopAvailable: false, desktopReason: "integrationDisabled", terminalReason: "integrationDisabled" });
  expect(antigravityLaunchReasons(same, t)).toEqual(["Antigravity integration with AskHuman is disabled"]);
  expect(antigravityLaunchReasons(status({ terminalReason: null }), t)).toEqual([]);
});

it("has a Chinese sentence for every reason the backend can give", () => {
  i18n.global.locale.value = "zh";
  const reasons = [
    "unsupportedPlatform", "devInstance", "appNotInstalled", "integrationDisabled", "mcpModeRequired", "pluginDisabled",
    "sidecarsNotAllowed", "yoloUnsupported", "cliNotDetected", "cliIntegrationNotConfigured", "cliTrackingDisabled",
    "cliTrackingNeedsUpdate", "cliTrackingNotConfigured", "terminalUnavailable", "cliLaunchNotAvailable",
  ] as const;
  for (const reason of reasons) {
    const text = t(`antigravityDesktop.launch.reasons.${reason}`);
    expect(text, reason).not.toContain("antigravityDesktop.");
    expect(text, reason).toMatch(/[\u4e00-\u9fff]/);
  }
  expect(antigravityLaunchDescription(status({}), t)).toBe("新任务将使用 Antigravity App");
});
