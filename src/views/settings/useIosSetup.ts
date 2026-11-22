// What the iOS App card shows is decided by facts the Mac already knows, never by a wizard position
// (docs/specs/ios-setup-experience.md §3.3): the settings, the connection, the paired phones, and the
// one thing only the person can say, that the app is installed. A pure function, so the rules can be
// tested.
import { channelState, channelTone, type ChannelState } from "../setup/channelDefs";

export interface IosSetupInput {
  enabled: boolean;
  host: string;
  username: string;
  /** Typed now, or stored from an earlier session. */
  passwordFilled: boolean;
  /** The gateway's answer; `null` while it is not running (or not asked yet). */
  gateway: { connected: boolean; lastError?: string | null } | null;
  /** Paired phones that are in use (not revoked). */
  phones: number;
  /** "I've installed it": a fact of the interface only, the Mac cannot see an install. */
  appInstalled: boolean;
}

export type IosStepNumber = 1 | 2 | 3;
export type IosStepStatus = "done" | "current" | "upcoming";

export interface IosSetup {
  /** Host, user name and password are all there. */
  configured: boolean;
  /** The gateway is running and connected to the broker. */
  connected: boolean;
  /** The status tag of the card header. */
  tag: ChannelState;
  tone: "ok" | "warn" | "muted";
  /** Whether each step is done, in order. */
  done: readonly [boolean, boolean, boolean];
  /** The first step that is not done; 4 when all are. */
  current: 1 | 2 | 3 | 4;
  /**
   * Fully set up and a phone is paired: the guide gives way to the usual sections. A broker that is
   * down for now (the Mac woke up, the service restarted) does not take the card back to the guide:
   * the phone stays paired, and the connection shows in the tag and in the box's status line.
   */
  ready: boolean;
}

export function deriveIosSetup(input: IosSetupInput): IosSetup {
  const configured =
    input.host.trim() !== "" && input.username.trim() !== "" && input.passwordFilled;
  const connected = input.gateway?.connected === true;
  const hasIssue =
    input.enabled &&
    configured &&
    input.gateway !== null &&
    !connected &&
    !!input.gateway.lastError;
  const tag = channelState({ enabled: input.enabled, configured, connected }, hasIssue);

  const phones = input.phones > 0;
  const done = [connected, phones || input.appInstalled, phones] as const;
  const firstOpen = done.findIndex((d) => !d);
  return {
    configured,
    connected,
    tag,
    tone: channelTone(tag),
    done,
    current: (firstOpen === -1 ? 4 : firstOpen + 1) as IosSetup["current"],
    ready: input.enabled && configured && phones,
  };
}

/** How one step is drawn: finished steps stay finished even while an earlier one is open again. */
export function stepStatus(setup: IosSetup, step: IosStepNumber): IosStepStatus {
  if (setup.done[step - 1]) return "done";
  return step === setup.current ? "current" : "upcoming";
}
