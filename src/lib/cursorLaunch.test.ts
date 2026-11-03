import { expect, it } from "vitest";
import { cursorLaunchDescription } from "./cursorLaunch";
import { i18n } from "../i18n";
import type { CursorLaunchStatus } from "./ipc";

const t = (key: string, values?: Record<string, string>) => i18n.global.t(key, values ?? {});

function status(patch: Partial<CursorLaunchStatus>): CursorLaunchStatus {
  return {
    preference: "terminal",
    target: "terminal",
    desktopAvailable: false,
    terminalAvailable: true,
    desktopReason: null,
    terminalReason: null,
    integrated: true,
    ...patch,
  };
}

it("names only the terminal's problem for someone who never chose the Agents window", () => {
  i18n.global.locale.value = "en";
  const text = cursorLaunchDescription(
    status({ target: null, terminalAvailable: false, desktopReason: "appNotRunning", terminalReason: "cliNotDetected" }),
    t,
  );
  expect(text).toBe("Unable to start: Cursor CLI was not detected");
});

it("names both problems when the Agents window was the choice", () => {
  i18n.global.locale.value = "en";
  const text = cursorLaunchDescription(
    status({ preference: "desktop", target: null, terminalAvailable: false, desktopReason: "appNotRunning", terminalReason: "cliNotDetected" }),
    t,
  );
  expect(text).toBe("Unable to start: Cursor is not running; Cursor CLI was not detected");
});

it("explains a fallback from the Agents window to the CLI", () => {
  i18n.global.locale.value = "en";
  expect(cursorLaunchDescription(status({ preference: "desktop", desktopReason: "multipleApps" }), t)).toBe(
    "New tasks will use the CLI (More than one Cursor process is running)",
  );
});
