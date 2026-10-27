import { describe, expect, it } from "vitest";
import type { IosTraffic } from "../../lib/types";
import { FREE_QUOTA_BYTES, brokerEstimate, formatBytes, quotaShare, trafficLevel } from "./iosTraffic";

function traffic(bytes: number): IosTraffic {
  return { month: "2026-10", bytesIn: bytes / 2, bytesOut: bytes / 2, msgsIn: 1, msgsOut: 1 };
}

describe("iosTraffic", () => {
  it("counts a message twice against the quota: published, then delivered", () => {
    expect(brokerEstimate(traffic(1000))).toBe(2000);
    expect(quotaShare(traffic(FREE_QUOTA_BYTES / 4))).toBeCloseTo(0.5);
  });

  it("warns from 80% and calls it full from 100% of the free quota", () => {
    expect(trafficLevel(traffic(FREE_QUOTA_BYTES * 0.39))).toBe("ok");
    expect(trafficLevel(traffic(FREE_QUOTA_BYTES * 0.4))).toBe("warn");
    expect(trafficLevel(traffic(FREE_QUOTA_BYTES * 0.49))).toBe("warn");
    expect(trafficLevel(traffic(FREE_QUOTA_BYTES * 0.5))).toBe("full");
  });

  it("formats sizes in the unit a person reads", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(1023)).toBe("1023 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(5 * 1024 * 1024)).toBe("5.0 MB");
    expect(formatBytes(250 * 1024 * 1024)).toBe("250 MB");
    expect(formatBytes(1.25 * 1024 ** 3)).toBe("1.3 GB");
  });
});
