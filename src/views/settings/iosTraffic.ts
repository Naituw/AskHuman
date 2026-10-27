// Monthly broker traffic of the iOS channel, as counted on this Mac (docs/specs/ios-channel.md §10).
// EMQX Serverless stops the service once the free quota is used up (the spend cap stays at 0), so the
// card warns before that happens. The figure is a local estimate: it only knows this Mac's messages.
import type { IosTraffic } from "../../lib/types";

/** EMQX Serverless free tier: 1 GiB per month, inbound plus outbound. */
export const FREE_QUOTA_BYTES = 1024 ** 3;
/** The card turns amber from this share of the free quota on. */
export const WARN_RATIO = 0.8;

export type TrafficLevel = "ok" | "warn" | "full";

/**
 * The broker counts a message when it is published and again when it is delivered, so a message
 * this Mac sends or receives costs about twice its size against the quota.
 */
export function brokerEstimate(traffic: IosTraffic): number {
  return 2 * (traffic.bytesIn + traffic.bytesOut);
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
