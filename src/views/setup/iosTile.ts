// What the wizard's iOS App tile shows (docs/specs/setup-ios-guidance.md §3.1). The card in Settings
// decides from the same facts with `deriveIosSetup`; the tile only cuts the card's tags a little finer,
// so the two never disagree about where the person stands.
import type { IosSetup } from "../settings/useIosSetup";

/**
 * One display per situation:
 * - `off`: nothing set up and the channel is off
 * - `incomplete`: on, but the broker settings are not all there
 * - `stopped`: set up, but turned off
 * - `issue`: on and complete, yet the gateway cannot reach the broker (and says why)
 * - `paired`: on and complete with at least one phone paired
 * - `await`: on and complete, broker connected, no phone yet
 * - `configured`: on and complete, no phone, and no connection to speak of (the daemon cannot be
 *   asked, or the broker is still being reached)
 */
export type IosTileKind = "off" | "incomplete" | "stopped" | "issue" | "paired" | "await" | "configured";

export function iosTileKind(setup: IosSetup): IosTileKind {
  switch (setup.tag) {
    case "unconfigured":
      return "off";
    case "incomplete":
      return "incomplete";
    case "configuredOff":
      return "stopped";
    case "issue":
      return "issue";
    default:
      // `ready` or `connected`: the phones decide, then the connection.
      if (setup.done[2]) return "paired";
      return setup.connected ? "await" : "configured";
  }
}

/** The tone of the status tag and the tile's border. */
export function iosTileTone(kind: IosTileKind): "ok" | "warn" | "muted" {
  switch (kind) {
    case "paired":
      return "ok";
    case "incomplete":
    case "issue":
    case "await":
      return "warn";
    default:
      return "muted";
  }
}

/**
 * Whether the main button starts the setup in the card (`channel#channel-ios#start`): the card is
 * still off, so the click that opens it is also the click on its "Start setup".
 */
export function iosTileStartsSetup(kind: IosTileKind): boolean {
  return kind === "off" || kind === "stopped";
}
