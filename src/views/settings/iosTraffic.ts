// Monthly broker-facing message traffic replicated across known devices (docs/specs/ios-traffic.md).
import type { IosTraffic, IosTrafficRecord } from "../../lib/types";

/** EMQX Serverless free tier: 1 GiB per month, inbound plus outbound. */
export const FREE_QUOTA_BYTES = 1024 ** 3;
/** The card turns amber from this share of the free quota on. */
export const WARN_RATIO = 0.8;

export type TrafficLevel = "ok" | "warn" | "full";

/**
 * Each device records its own broker-facing leg. Adding the endpoint records counts each leg once.
 */
export function brokerEstimate(traffic: IosTraffic): number {
  return traffic.bytesIn + traffic.bytesOut;
}

export function quotaShare(traffic: IosTraffic): number {
  return brokerEstimate(traffic) / FREE_QUOTA_BYTES;
}

export function trafficLevel(traffic: IosTraffic): TrafficLevel {
  const share = quotaShare(traffic);
  if (share >= 1) return "full";
  return share >= WARN_RATIO ? "warn" : "ok";
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 ? value.toFixed(0) : value.toFixed(1)} ${units[unit]}`;
}

/** Sum each owner's cumulative record exactly once. */
export function aggregateTraffic(records: IosTrafficRecord[], month: string): IosTraffic {
  const owners = new Map<string, IosTrafficRecord>();
  for (const record of records) {
    if (record.month !== month) continue;
    const key = `${record.broker}|${record.deviceKind ?? "mac"}|${record.macId}`;
    const previous = owners.get(key);
    owners.set(key, previous ? {
      ...record,
      bytesIn: Math.max(previous.bytesIn, record.bytesIn),
      bytesOut: Math.max(previous.bytesOut, record.bytesOut),
      msgsIn: Math.max(previous.msgsIn, record.msgsIn),
      msgsOut: Math.max(previous.msgsOut, record.msgsOut),
    } : record);
  }
  const result: IosTraffic = { month, bytesIn: 0, bytesOut: 0, msgsIn: 0, msgsOut: 0 };
  for (const record of owners.values()) {
    result.bytesIn += record.bytesIn;
    result.bytesOut += record.bytesOut;
    result.msgsIn += record.msgsIn;
    result.msgsOut += record.msgsOut;
  }
  return result;
}

export function hasCompleteTrafficHistory(records: IosTrafficRecord[], month: string): boolean {
  const start = Date.parse(`${month}-01T00:00:00Z`);
  return records.some((record) => record.deviceKind === "phone") && records.every((record) =>
    record.month === month && record.recordedFrom != null && record.recordedFrom <= start && !record.uncertain,
  );
}
