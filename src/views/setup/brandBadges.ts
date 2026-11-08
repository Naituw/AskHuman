// Badge art for the wizard's tiles, drawn the way the iOS app draws an Agent
// (`ios/AskHuman/Design/AgentBadge.swift`) and the way the README shows the products: a rounded tile
// with the brand's mark on it, either a white mark on a tile in the brand's color or the mark in its
// own colors on a white tile. The Agents' marks are the iOS app's own SVG sources, imported from
// there so a logo exists once; the rest live in `./logos`. Sources and licenses: `./logos/NOTICE.md`.
//
// An id without an entry falls back to the first letter of its name on a grey tile, so a further
// Agent or channel never renders blank.

import antigravitySvg from "../../../ios/Tools/agent-logos/antigravity.svg?raw";
import claudeSvg from "../../../ios/Tools/agent-logos/claude.svg?raw";
import codexSvg from "../../../ios/Tools/agent-logos/codex.svg?raw";
import cursorSvg from "../../../ios/Tools/agent-logos/cursor.svg?raw";
import grokSvg from "../../../ios/Tools/agent-logos/grok.svg?raw";
import piSvg from "../../../ios/Tools/agent-logos/pi.svg?raw";
import dingtalkSvg from "./logos/dingtalk.svg?raw";
import feishuSvg from "./logos/feishu.svg?raw";
import phoneSvg from "./logos/phone.svg?raw";
import slackSvg from "./logos/slack.svg?raw";
import telegramSvg from "./logos/telegram.svg?raw";
import windowSvg from "./logos/window.svg?raw";

export interface BadgeSpec {
  /** CSS `background` of the tile. */
  tile: string;
  /** Colour of the hairline round the tile: light on dark tiles, dark on a white one. */
  hairline: string;
  mark: {
    /** SVG source: one black shape on its own grid, or a drawing in its own colours. */
    svg: string;
    /** Width of the mark relative to the tile. */
    scale: number;
    /**
     * CSS `background` painted through the shape of a one-colour mark. Absent: the SVG keeps its
     * own colours.
     */
    paint?: string;
  };
}

const WHITE = "#fff";
const LIGHT_HAIRLINE = "rgba(255, 255, 255, 0.14)";
const DARK_HAIRLINE = "rgba(0, 0, 0, 0.12)";

// The Agents' tiles mirror `AgentStyle` in AgentBadge.swift; keep the two in step.
const NIGHT = "linear-gradient(180deg, #3b3b40, #17171a)";

export const BADGES: Readonly<Record<string, BadgeSpec>> = {
  // Agents
  cursor: {
    tile: NIGHT,
    hairline: LIGHT_HAIRLINE,
    mark: { svg: cursorSvg, scale: 0.52, paint: WHITE },
  },
  claude: {
    tile: "linear-gradient(180deg, #dc8465, #d97857)",
    hairline: LIGHT_HAIRLINE,
    mark: { svg: claudeSvg, scale: 0.56, paint: WHITE },
  },
  // The Codex app icon: a white tile with the cloud in its violet-to-blue gradient.
  codex: {
    tile: WHITE,
    hairline: DARK_HAIRLINE,
    mark: {
      svg: codexSvg,
      scale: 0.68,
      paint: "linear-gradient(180deg, #b1a7ff, #7a9dff 50%, #3941ff)",
    },
  },
  grok: {
    tile: "linear-gradient(180deg, #1c1c1c, #080808)",
    hairline: LIGHT_HAIRLINE,
    mark: { svg: grokSvg, scale: 0.52, paint: WHITE },
  },
  pi: {
    tile: NIGHT,
    hairline: LIGHT_HAIRLINE,
    mark: { svg: piSvg, scale: 0.48 },
  },
  antigravity: {
    tile: WHITE,
    hairline: DARK_HAIRLINE,
    mark: { svg: antigravitySvg, scale: 0.62 },
  },
  // Remote channels
  telegram: {
    tile: WHITE,
    hairline: DARK_HAIRLINE,
    mark: { svg: telegramSvg, scale: 0.8 },
  },
  slack: {
    tile: WHITE,
    hairline: DARK_HAIRLINE,
    mark: { svg: slackSvg, scale: 0.64 },
  },
  dingding: {
    tile: "linear-gradient(180deg, #4ea6ff, #2f6cf6)",
    hairline: LIGHT_HAIRLINE,
    mark: { svg: dingtalkSvg, scale: 0.6, paint: WHITE },
  },
  // Feishu, called Lark outside China: the same bird.
  feishu: {
    tile: WHITE,
    hairline: DARK_HAIRLINE,
    mark: { svg: feishuSvg, scale: 0.7 },
  },
  // The iPhone app, which is itself a way to answer remotely.
  ios: {
    tile: "linear-gradient(180deg, #8f8f96, #5b5b61)",
    hairline: LIGHT_HAIRLINE,
    mark: { svg: phoneSvg, scale: 0.5, paint: WHITE },
  },
  // The local popup window, the one "channel" that is not a remote service (Settings → Channels).
  popup: {
    tile: "linear-gradient(180deg, #4da3ff, #0a6cdb)",
    hairline: LIGHT_HAIRLINE,
    mark: { svg: windowSvg, scale: 0.52, paint: WHITE },
  },
};

/** The tile of an id without an entry. */
export const FALLBACK_TILE = "linear-gradient(145deg, #6e6e73, #48484a)";

const urls = new Map<string, string>();

/** A CSS `url()` that carries the SVG itself, so a badge needs no request and no file lookup. */
export function svgUrl(svg: string): string {
  let url = urls.get(svg);
  if (url === undefined) {
    url = `url("data:image/svg+xml,${encodeURIComponent(svg.trim())}")`;
    urls.set(svg, url);
  }
  return url;
}
