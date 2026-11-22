import { afterEach, describe, expect, it, vi } from "vitest";
import {
  actOnSettingsTarget,
  revealSettingsTarget,
  SETTINGS_ACTION_EVENT,
  SETTINGS_REVEAL_EVENT,
} from "./reveal";

afterEach(() => {
  document.body.innerHTML = "";
});

function card() {
  const outer = document.createElement("section");
  const el = document.createElement("div");
  outer.appendChild(el);
  document.body.appendChild(outer);
  return { outer, el };
}

describe("revealSettingsTarget", () => {
  it("asks whatever folds the target away to open, on the target and up its ancestors", async () => {
    const { outer, el } = card();
    const asked = vi.fn();
    outer.addEventListener(SETTINGS_REVEAL_EVENT, asked);
    await revealSettingsTarget(el);
    expect(asked).toHaveBeenCalledTimes(1);
  });

  it("does nothing for a target that is not there", async () => {
    await expect(revealSettingsTarget(null)).resolves.toBeUndefined();
  });
});

describe("actOnSettingsTarget", () => {
  it("announces the action on the target, where its owner (or an ancestor) can hear it", () => {
    const { outer, el } = card();
    const heard = vi.fn();
    outer.addEventListener(SETTINGS_ACTION_EVENT, (event) => heard((event as CustomEvent).detail));
    actOnSettingsTarget(el, "start");
    expect(heard).toHaveBeenCalledTimes(1);
    expect(heard).toHaveBeenCalledWith({ action: "start" });
  });

  it.each([
    ["has no action", (el: Element) => actOnSettingsTarget(el, "")],
    ["is not there", () => actOnSettingsTarget(null, "start")],
  ])("announces nothing when the target %s", (_name, act) => {
    const { outer, el } = card();
    const heard = vi.fn();
    outer.addEventListener(SETTINGS_ACTION_EVENT, heard);
    act(el);
    expect(heard).not.toHaveBeenCalled();
  });
});
