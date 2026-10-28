import { describe, expect, it } from "vitest";
import { alertAfterChoices, idleDuration, splitDuration } from "./iosAlertTiming";

describe("iosAlertTiming", () => {
  it("splits a configured wait into the largest unit that divides it", () => {
    expect(splitDuration(300)).toEqual({ n: 5, unit: "minutes" });
    expect(splitDuration(900)).toEqual({ n: 15, unit: "minutes" });
    expect(splitDuration(3600)).toEqual({ n: 1, unit: "hours" });
    expect(splitDuration(7200)).toEqual({ n: 2, unit: "hours" });
    expect(splitDuration(5400)).toEqual({ n: 90, unit: "minutes" });
    expect(splitDuration(45)).toEqual({ n: 45, unit: "seconds" });
    expect(splitDuration(91)).toEqual({ n: 91, unit: "seconds" });
  });

  it("rounds the idle time down to seconds, minutes or hours", () => {
    expect(idleDuration(0)).toEqual({ n: 0, unit: "seconds" });
    expect(idleDuration(59)).toEqual({ n: 59, unit: "seconds" });
    expect(idleDuration(60)).toEqual({ n: 1, unit: "minutes" });
    expect(idleDuration(179)).toEqual({ n: 2, unit: "minutes" });
    expect(idleDuration(3599)).toEqual({ n: 59, unit: "minutes" });
    expect(idleDuration(3600)).toEqual({ n: 1, unit: "hours" });
    expect(idleDuration(2 * 3600 + 1800)).toEqual({ n: 2, unit: "hours" });
  });

  it("lists the presets, plus a hand-edited value in its place", () => {
    expect(alertAfterChoices(900)).toEqual([0, 300, 900, 1800, 3600]);
    expect(alertAfterChoices(0)).toEqual([0, 300, 900, 1800, 3600]);
    expect(alertAfterChoices(600)).toEqual([0, 300, 600, 900, 1800, 3600]);
    expect(alertAfterChoices(86_400)).toEqual([0, 300, 900, 1800, 3600, 86_400]);
  });

  it("ignores a value that cannot be a wait", () => {
    expect(alertAfterChoices(Number.NaN)).toEqual([0, 300, 900, 1800, 3600]);
    expect(alertAfterChoices(-5)).toEqual([0, 300, 900, 1800, 3600]);
  });
});
