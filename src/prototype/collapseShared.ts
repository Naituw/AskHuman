// Types, defaults and event names shared by the mock answer window (stage) and the demo controller.

export type Curve = [number, number, number, number];
export type CurveName = "emphasized" | "standard" | "quint" | "expo" | "inout" | "ios" | "custom";

/** Cubic-bezier presets (CSS semantics). Every one is monotonic: no overshoot or bounce (user preference). */
export const CURVES: Record<Exclude<CurveName, "custom">, Curve> = {
  emphasized: [0.2, 0, 0, 1],
  standard: [0.4, 0, 0.2, 1],
  quint: [0.22, 1, 0.36, 1],
  expo: [0.16, 1, 0.3, 1],
  inout: [0.65, 0, 0.35, 1],
  ios: [0.32, 0.72, 0, 1],
};

export const CURVE_LABELS: Record<CurveName, string> = {
  emphasized: "强调减速 (.2,0,0,1)",
  standard: "标准 (.4,0,.2,1)",
  quint: "五次方缓出 (.22,1,.36,1)",
  expo: "指数缓出 (.16,1,.3,1)",
  inout: "三次缓入缓出 (.65,0,.35,1)",
  ios: "iOS 面板 (.32,.72,0,1)",
  custom: "自定义",
};

export interface Tuning {
  /** Small-window form: mini list (the Sidebar contracts) or a capsule. */
  form: "list" | "capsule";
  /** How conspicuous the small window is. */
  salience: "low" | "mid" | "high";
  /** Visible request rows in the small list before it scrolls. */
  rows: number;
  /** Compact drops the agent / session line of every row while small. */
  density: "compact" | "same";
  /** Secondary rows keep their position or sink below the primary rows of their project. */
  secondaryOrder: "stay" | "bottom";
  /** Traffic lights fade out in the small window or stay. */
  lights: "hide" | "keep";
  sidebarWidth: number;
  collapseMs: number;
  expandMs: number;
  collapseCurve: CurveName;
  expandCurve: CurveName;
  custom: Curve;
  /** Which axis of the window leads. */
  stagger: "together" | "height-first" | "width-first";
  /** 0 = lagging axis starts after the leader is half done, 1 = fully simultaneous. */
  staggerOverlap: number;
  /** Multiplies every duration (slow motion for inspection). */
  slow: number;
  reduceMotion: boolean;
  /** Window corner radius used to inset the accent ring. */
  radius: number;
  /** Requests created when a round starts. */
  initialCount: number;
  /** Sidebar row action: archive box (hover icon -> "稍后答" label -> hover "取消稍后答"), text pill, clock / undo icons, or sink / raise chevrons. */
  rowAction: "archive" | "text" | "clock" | "chevron";
  /** C23: leaving AskHuman while the expanded window shows only "稍后答" requests shrinks it back to the small window. */
  autoCollapseOnLeave: boolean;
  /** Play the arrival bubble when a new request shows up (product: popup-arrival-envelope.md). */
  arrivalBubble: boolean;
  /** After a new request expands the small window: show it (C4) or keep the body the small window would open. */
  arrivalBody: "new" | "keep";
  /** C31: how long files carried over the small window rest on it before it opens (0 = at once). */
  dragExpandMs: number;
  /** C28: how long files carried over a Sidebar row rest on it before the body switches to that request (0 = at once). */
  dragSwitchMs: number;
  /** C28: a drag that opened the small window and leaves again without dropping shrinks the window back. */
  dragSpringBack: boolean;
  /** C28: dropping files brings AskHuman to the front, so leaving it later shrinks a peek again (C23). */
  dropActivates: boolean;
  /** C32: how a request with several questions lays them out (the product's default, or its experimental vertical list). */
  questionLayout: "sequential" | "vertical";
}

export const DEFAULT_TUNING: Tuning = {
  form: "list",
  salience: "mid",
  rows: 3,
  density: "compact",
  secondaryOrder: "stay",
  lights: "hide",
  sidebarWidth: 260,
  collapseMs: 440,
  expandMs: 480,
  collapseCurve: "emphasized",
  expandCurve: "emphasized",
  custom: [0.2, 0, 0, 1],
  stagger: "together",
  staggerOverlap: 0.6,
  slow: 1,
  reduceMotion: false,
  radius: 16,
  initialCount: 1,
  rowAction: "archive",
  autoCollapseOnLeave: true,
  arrivalBubble: true,
  arrivalBody: "new",
  dragExpandMs: 500,
  dragSwitchMs: 250,
  dragSpringBack: true,
  dropActivates: true,
  questionLayout: "sequential",
};

export function curveOf(name: CurveName, custom: Curve): Curve {
  return name === "custom" ? custom : CURVES[name];
}

export function cssCurve(curve: Curve): string {
  return `cubic-bezier(${curve.join(",")})`;
}

export const TUNING_KEY = "askhuman.collapse-demo.tuning.v1";

export function loadTuning(): Tuning {
  try {
    const stored = JSON.parse(localStorage.getItem(TUNING_KEY) ?? "null");
    if (stored && typeof stored === "object") {
      // "text" was the default before the archive-box action existed: let that stored value follow the new default.
      if (stored.rowAction === "text") delete stored.rowAction;
      return { ...DEFAULT_TUNING, ...stored };
    }
  } catch { /* fall through to defaults */ }
  return { ...DEFAULT_TUNING };
}

export const EVENTS = {
  tuning: "collapse-tuning",
  action: "collapse-action",
  state: "collapse-state",
  report: "collapse-report",
  hello: "collapse-hello",
} as const;

export type DemoAction =
  | { type: "arrive"; kind: "ask" | "permission" | "stop"; other?: boolean; multi?: boolean }
  | { type: "batch" }
  | { type: "delayed"; seconds: number }
  | { type: "delayedEnd"; seconds: number }
  | { type: "leave" }
  | { type: "shrink" }
  | { type: "putAway" }
  | { type: "markAll" }
  | { type: "unmarkAll" }
  | { type: "answer" }
  | { type: "endActive" }
  | { type: "endLast" }
  | { type: "trayFocus" }
  | { type: "expandDefault" }
  | { type: "reset"; count: number }
  /** A ready-made scene for trying drags: three requests (the first has three questions), all 稍后答, small window. */
  | { type: "dragScene" }
  | { type: "pin"; on: boolean };

/**
 * Files dragged over the window, as Tauri reports them (`onDragDropEvent`). wry/macOS gives the pointer in window
 * points: x relative to the window's left edge, y measured down from the top of the *webview frame*.
 */
export type DragPayload =
  | { type: "enter"; paths: string[]; position: { x: number; y: number } }
  | { type: "over"; position: { x: number; y: number } }
  | { type: "drop"; paths: string[]; position: { x: number; y: number } }
  | { type: "leave" };

export interface StateSnapshot {
  view: string;
  phase: string;
  active: number | null;
  pinned: boolean;
  requests: { id: number; title: string; secondary: boolean; visited: boolean }[];
  closed: boolean;
}

/** Window facts read from AppKit (`demo_info`). Coordinates are points; `top` is the window's top edge in screen space. */
export interface NativeInfo {
  x: number; top: number; width: number; height: number; scale: number;
  screenX: number; screenTop: number; screenW: number; screenH: number;
  level: number; alwaysOnTop: boolean; minW: number; minH: number;
  lightsHidden: boolean; resizable: boolean;
  canvasInstalled: boolean; canvasResponsive: boolean; canvasX: number;
  viewW: number; viewH: number; visible: boolean; key: boolean; lightsAlpha: number; hasShadow: boolean;
}

/** Frame-by-frame measurement of one native animation (`demo_animate`). */
export interface NativeReport {
  frames: number; maxGapMs: number; meanGapMs: number; p95GapMs: number; dropped: number; wallMs: number;
  finalX: number; finalTop: number; finalW: number; finalH: number;
  cancelled: boolean; maxErrorPx: number; refreshHz: number;
  canvasW: number; canvasH: number; fromX: number; fromTop: number; fromW: number; fromH: number;
  /** Delay before the first display-link tick, per-frame main-thread cost, and the first gaps (ms). */
  startDelayMs: number; maxCostMs: number; meanCostMs: number; gaps: number[];
  /** Per tick: callback start / placement end, ms after the vsync that woke the display link. */
  lates: number[]; ends: number[];
  /** Per tick: the display time (ms, mach clock) the frame is meant for. */
  targets: number[];
  /** Per tick: the window size placed (points). */
  widths: number[]; heights: number[];
}

/** Pixel comparison of one screen-space region in two window captures (`demo_diff`). */
export interface RegionDiff {
  meanAbs: number; changedFraction: number; stddevA: number; stddevB: number; widthPx: number; heightPx: number;
}

export interface TransitionReport {
  kind: string;
  wantedMs: number;
  frames: number;
  maxGapMs: number;
  p95GapMs: number;
  meanGapMs: number;
  dropped: number;
  wallMs: number;
  maxErrorPx: number;
  refreshHz: number;
  cancelled: boolean;
  from: string;
  to: string;
}
