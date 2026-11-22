import { describe, expect, it } from "vitest";
import { parseSettingsTarget } from "./settingsTarget";

describe("parseSettingsTarget", () => {
  it.each([
    ["channel", { tab: "channel", element: "", action: "" }],
    ["integration#lifecycle-claude", { tab: "integration", element: "lifecycle-claude", action: "" }],
    ["channel#channel-ios#start", { tab: "channel", element: "channel-ios", action: "start" }],
    ["channel#channel-ios#", { tab: "channel", element: "channel-ios", action: "" }],
    ["channel##start", { tab: "channel", element: "", action: "start" }],
    ["", { tab: "", element: "", action: "" }],
  ])("reads %j as tab, element and action", (raw, expected) => {
    expect(parseSettingsTarget(raw)).toEqual(expected);
  });

  it("leaves the tab for the caller to check", () => {
    expect(parseSettingsTarget("nowhere#x").tab).toBe("nowhere");
  });
});
