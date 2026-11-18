import { describe, expect, it } from "vitest";
import type { IosTraffic } from "../../lib/types";
import { FREE_QUOTA_BYTES, GIGABYTE, aggregateTraffic, hasCompleteTrafficHistory, brokerEstimate, formatBytes, quotaShare, trafficReminder } from "./iosTraffic";

function traffic(bytes: number): IosTraffic {
  return { month: "2026-10", bytesIn: bytes / 2, bytesOut: bytes / 2, msgsIn: 1, msgsOut: 1 };
}

describe("iosTraffic", () => {
  it("adds each recorded broker-facing leg once", () => {
    expect(brokerEstimate(traffic(1000))).toBe(1000);
    expect(quotaShare(traffic(FREE_QUOTA_BYTES / 4))).toBeCloseTo(0.25);
  });

  // The same cases as the phone's TrafficReminderTests: one rule on both screens.
  describe("trafficReminder", () => {
    const lead = (gigabytes: number, step = 1) => gigabytes * GIGABYTE - Math.floor((step * GIGABYTE) / 5);

    it("says nothing until the last fifth before the first GB", () => {
      expect(trafficReminder(0)).toBeNull();
      expect(trafficReminder(GIGABYTE / 2)).toBeNull();
      expect(trafficReminder(lead(1) - 1)).toBeNull();
    });

    it("is close to a GB in the last fifth before it and has reached it from there on", () => {
      expect(trafficReminder(lead(1))).toEqual({ gigabytes: 1, reached: false });
      expect(trafficReminder(GIGABYTE - 1)).toEqual({ gigabytes: 1, reached: false });
      expect(trafficReminder(GIGABYTE)).toEqual({ gigabytes: 1, reached: true });
      expect(trafficReminder(1.5 * GIGABYTE)).toEqual({ gigabytes: 1, reached: true });
      expect(trafficReminder(lead(2) - 1)).toEqual({ gigabytes: 1, reached: true });
    });

    it("has a reminder for every GB", () => {
      expect(trafficReminder(lead(2))).toEqual({ gigabytes: 2, reached: false });
      expect(trafficReminder(2 * GIGABYTE)).toEqual({ gigabytes: 2, reached: true });
      expect(trafficReminder(lead(3))).toEqual({ gigabytes: 3, reached: false });
      expect(trafficReminder(7.3 * GIGABYTE)).toEqual({ gigabytes: 7, reached: true });
    });

    it("keeps the lead of a fifth of the step for a larger step", () => {
      expect(trafficReminder(lead(2, 2) - 1, 2)).toBeNull();
      expect(trafficReminder(lead(2, 2), 2)).toEqual({ gigabytes: 2, reached: false });
      expect(trafficReminder(3 * GIGABYTE, 2)).toEqual({ gigabytes: 2, reached: true });
      expect(trafficReminder(lead(4, 2), 2)).toEqual({ gigabytes: 4, reached: false });
      expect(trafficReminder(4 * GIGABYTE - 1, 5)).toBeNull();
      expect(trafficReminder(4 * GIGABYTE, 5)).toEqual({ gigabytes: 5, reached: false });
    });
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

it("merges repeated owner snapshots before summing the known Macs", () => {
  const record = (macId: string, bytesOut: number) => ({
    broker: "broker.test", month: "2026-11", macId, updatedAt: 1,
    bytesIn: 0, bytesOut, msgsIn: 0, msgsOut: 1,
  });
  const result = aggregateTraffic([record("a", 100), record("b", 50), record("a", 80)], "2026-11");
  expect(result.bytesOut).toBe(150);
  expect(brokerEstimate(result)).toBe(150);
  expect(aggregateTraffic([record("a", 100)], "2026-12").bytesOut).toBe(0);
});

it("marks legacy history and missing endpoint coverage as incomplete", () => {
  const month = "2026-11";
  const base = { broker: "b", month, bytesIn: 0, bytesOut: 100, msgsIn: 0, msgsOut: 1, updatedAt: 1,
    recordedFrom: Date.parse("2026-11-01T00:00:00Z") };
  const records = [{ ...base, macId: "a", deviceKind: "mac" as const }, { ...base, macId: "p", deviceKind: "phone" as const }];
  expect(hasCompleteTrafficHistory(records, month)).toBe(true);
  expect(brokerEstimate(aggregateTraffic(records, month))).toBe(200);
  expect(hasCompleteTrafficHistory([records[0]], month)).toBe(false);
  expect(hasCompleteTrafficHistory([records[0], { ...records[1], recordedFrom: null }], month)).toBe(false);
  expect(hasCompleteTrafficHistory([records[0], { ...records[1], uncertain: true }], month)).toBe(false);
});
