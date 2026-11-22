// Addresses the iOS App card points at (docs/specs/ios-setup-experience.md §3.1, §5).

const REPO_WIKI = "https://github.com/Naituw/AskHuman/blob/main/docs/wiki";

/**
 * The page that gets a phone into the TestFlight beta (docs/specs/ios-open-access.md OA13): it
 * carries the invitation link, so the address printed here never has to change when the link does.
 */
export const IOS_JOIN_URL = "https://naituw.github.io/AskHuman/";

/** The setup guide of the iOS App channel in the reader's language. */
export function iosSetupGuideUrl(zh: boolean): string {
  return `${REPO_WIKI}/${zh ? "ios-setup.md" : "ios-setup.en.md"}`;
}
