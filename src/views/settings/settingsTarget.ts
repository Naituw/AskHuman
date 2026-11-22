// The address of a place in the settings window: `tab`, `tab#elementId` or `tab#elementId#action`.
// It is the argument of `open_settings`, the payload of the `settings-goto-tab` event and the initial
// `?tab=` of a new window. The tab is switched to and the element scrolled to and highlighted
// (docs/specs/gui-agent-task-launch.md G5); the action is something the element's owner does once the
// jump has brought it on screen (docs/specs/setup-ios-guidance.md D7).

export interface SettingsTarget {
  /** Not checked here: the caller knows which tabs exist. */
  tab: string;
  /** Empty when the address only names a tab. */
  element: string;
  /** Empty when the address only points at the element. */
  action: string;
}

export function parseSettingsTarget(raw: string): SettingsTarget {
  const [tab = "", element = "", action = ""] = raw.split("#");
  return { tab, element, action };
}
