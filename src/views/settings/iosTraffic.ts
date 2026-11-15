// Monthly broker-facing message traffic replicated across known devices (docs/specs/ios-traffic.md).
import type { IosTraffic, IosTrafficRecord } from "../../lib/types";

/** EMQX Serverless free tier: 1 GiB per month, inbound plus outbound. */
export const FREE_QUOTA_BYTES = 1024 ** 3;
/** A GB as the broker counts it: 1024³ bytes. */
export const GIGABYTE = 1024 ** 3;

/**
 * Each device records its own broker-facing leg. Adding the endpoint records counts each leg once.
 */
export function brokerEstimate(traffic: IosTraffic): number {
  return traffic.bytesIn + traffic.bytesOut;
}

export function quotaShare(traffic: IosTraffic): number {
  return brokerEstimate(traffic) / FREE_QUOTA_BYTES;
}

/** What the card says about this month's data use: close to a multiple of the step, or past it. */
export interface TrafficReminder {
  /** The multiple of the step the use is close to or past, in GB. */
  gigabytes: number;
  /** The use has reached `gigabytes`; otherwise it is only close: in the last fifth before it. */
  reached: boolean;
}

/**
 * The reminder for `bytes` used this month, at every `step` GB; null below the lead of the first multiple. It counts and
 * says nothing of the broker's plan. The last fifth before each multiple counts as close to it, so the first reminder of a
 * 1 GB step comes at 80%. The phone's Inbox follows the same rule (`TrafficReminder.due`, docs/specs/ios-traffic.md); this
 * card has no step to choose and no way to close it: it is a line under the usage, not a notice.
 */
export function trafficReminder(bytes: number, step = 1): TrafficReminder | null {
  const unit = step * GIGABYTE;
  const whole = Math.floor(bytes / unit);
  // `unit - floor(unit / 5)` is the first whole byte at or past 80% of the step.
  if (bytes - whole * unit >= unit - Math.floor(unit / 5)) {
    return { gigabytes: (whole + 1) * step, reached: false };
  }
  return whole > 0 ? { gigabytes: whole * step, reached: true } : null;
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
