// Badge art for the wizard's tiles: a rounded square in a product-like hue with one simple glyph.
// These are original, simplified marks, not the vendors' logos. An id without an entry falls back
// to the first letter of its name, so a further Agent or channel never renders blank.

export interface BadgeSpec {
  /** Gradient stops, top-left to bottom-right. */
  from: string;
  to: string;
  /** Stroked path data on a 24x24 canvas; takes precedence over `glyph`. */
  paths?: string[];
  /** A text glyph, used when there is no drawing. */
  glyph?: string;
}

export const BADGES: Readonly<Record<string, BadgeSpec>> = {
  // Agents
  cursor: {
    from: "#4b4b52",
    to: "#18181b",
    paths: ["M4.5 4.5l6.6 15.9 2.3-6.9 6.9-2.3L4.5 4.5z", "M13.6 13.6l5.2 5.2"],
  },
  claude: {
    from: "#e48d69",
    to: "#c75f3a",
    paths: ["M12 3.5v17", "M4.6 7.75l14.8 8.5", "M4.6 16.25l14.8-8.5"],
  },
  codex: {
    from: "#1fbb91",
    to: "#0d8465",
    paths: ["M6 8l4.5 4L6 16", "M13 16.5h5.5"],
  },
  grok: { from: "#5b5c63", to: "#1a1a1d", glyph: "G" },
  antigravity: { from: "#6aa3ff", to: "#3d6fe6", glyph: "A" },
  pi: { from: "#a083ff", to: "#6d4ae2", glyph: "π" },
  // Remote channels
  telegram: {
    from: "#3ab0e6",
    to: "#1d94c8",
    paths: ["M21.5 3L10.8 13.7", "M21.5 3l-6.7 18-4-8.3L2.5 8.7 21.5 3z"],
  },
  slack: { from: "#6a2a72", to: "#3d0f3f", glyph: "#" },
  dingding: { from: "#40a4ff", to: "#1670f0", glyph: "钉" },
  feishu: { from: "#1fd8c0", to: "#2f6bf2", glyph: "飞" },
  ios: {
    from: "#8f8f96",
    to: "#5b5b61",
    paths: [
      "M7.5 2.5h9a2 2 0 0 1 2 2v15a2 2 0 0 1-2 2h-9a2 2 0 0 1-2-2v-15a2 2 0 0 1 2-2z",
      "M12 18h.01",
    ],
  },
};
