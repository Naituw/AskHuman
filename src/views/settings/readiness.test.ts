import { describe, expect, it } from "vitest";
import { readinessIssues, readinessState } from "./readiness";

const ok = { binaryReady: true, lifecycleReady: true, integrationReady: true };

describe("readinessIssues", () => {
  it("lists nothing for an Agent that has everything", () => {
    expect(readinessIssues(ok)).toEqual([]);
  });

  it.each([
    [{ binaryReady: false }, ["binary"]],
    [{ lifecycleReady: false }, ["lifecycle"]],
    [{ integrationReady: false }, ["integration"]],
  ])("lists only the one thing that is missing: %j", (missing, expected) => {
    expect(readinessIssues({ ...ok, ...missing })).toEqual(expected);
  });

  it("lists several in the order they are fixed, not the order they were found", () => {
    expect(
      readinessIssues({ binaryReady: false, lifecycleReady: false, integrationReady: false }),
    ).toEqual(["binary", "lifecycle", "integration"]);
    expect(readinessIssues({ ...ok, lifecycleReady: false, integrationReady: false })).toEqual([
      "lifecycle",
      "integration",
    ]);
  });
});

describe("readinessState", () => {
  it("is ready for a supported Agent that has everything", () => {
    expect(readinessState({ ready: true, launchSupported: true })).toBe("ready");
  });

  it("is not ready for a supported Agent that still lacks something", () => {
    expect(readinessState({ ready: false, launchSupported: true })).toBe("notReady");
  });

  it("is unsupported for an Agent that cannot start tasks, whatever its conditions say", () => {
    // Nothing can be fixed for it, so it is neither "ready" nor "not ready".
    expect(readinessState({ ready: false, launchSupported: false })).toBe("unsupported");
    expect(readinessState({ ready: true, launchSupported: false })).toBe("unsupported");
  });
});
