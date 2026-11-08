import { expect, it } from "vitest";
import { cursorLaunchDescription, cursorLaunchReasons } from "./cursorLaunch";
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

it("names both problems when neither route works, the one that goes first leading", () => {
  i18n.global.locale.value = "en";
  const stuck = { target: null, terminalAvailable: false, desktopReason: "appNotRunning", terminalReason: "cliNotDetected" } as const;
  // Terminal first: whoever prefers the CLI is told about the CLI before the window.
  expect(cursorLaunchDescription(status(stuck), t)).toBe(
    "Unable to start: Cursor CLI was not detected; Cursor is not running",
  );
  expect(cursorLaunchDescription(status({ ...stuck, preference: "desktop" }), t)).toBe(
    "Unable to start: Cursor is not running; Cursor CLI was not detected",
  );
});

it("lists the reasons on their own, preferred route first, without repeating one", () => {
  i18n.global.locale.value = "en";
  const stuck = { target: null, terminalAvailable: false, desktopReason: "appNotRunning", terminalReason: "cliNotDetected" } as const;
  expect(cursorLaunchReasons(status(stuck), t)).toEqual(["Cursor CLI was not detected", "Cursor is not running"]);
  expect(cursorLaunchReasons(status({ ...stuck, preference: "desktop" }), t)).toEqual([
    "Cursor is not running",
    "Cursor CLI was not detected",
  ]);
  expect(
    cursorLaunchReasons(status({ ...stuck, desktopReason: "cliNotDetected" }), t),
  ).toEqual(["Cursor CLI was not detected"]);
  expect(cursorLaunchReasons(status({}), t)).toEqual([]);
});

it("explains a fallback from the Agents window to the CLI", () => {
  i18n.global.locale.value = "en";
  expect(cursorLaunchDescription(status({ preference: "desktop", desktopReason: "multipleApps" }), t)).toBe(
    "New tasks will use the CLI (More than one Cursor process is running)",
  );
});

it("explains a fallback from the CLI to the Agents window", () => {
  i18n.global.locale.value = "en";
  expect(
    cursorLaunchDescription(
      status({ target: "desktop", terminalAvailable: false, desktopAvailable: true, terminalReason: "cliNotDetected" }),
      t,
    ),
  ).toBe("New tasks will use Cursor's Agents window (Cursor CLI was not detected)");
});
