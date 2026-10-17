<script setup lang="ts">
// Mock answer window for the collapsible inbox prototype. It owns the mock inbox state (the pure
// model in collapseModel.ts) and drives the real NSWindow through the demo_* commands.
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { getCurrentWindow } from "@tauri-apps/api/window";
import SessionIcon from "../components/SessionIcon.vue";
import UnreadRipple from "../views/popup/UnreadRipple.vue";
import InboxArrivalNotice from "../views/popup/InboxArrivalNotice.vue";
import { arrivalTiming, type ArrivalEnd, type ArrivalGeometry } from "../views/popup/inboxArrival";
import {
  arrive, browse, collapseWindow, defaultOpenTarget, emptyState, finish, markSecondary, orderedRequests,
  putAway, sidebarVisible, unmark,
  type CQuestion, type CRequest, type InboxState, type Kind, type Outcome, type View,
} from "./collapseModel";
import { runScenario, type StageApi } from "./collapseAuto";
import { runBurst, runVideo } from "./collapseBurst";
import {
  cssCurve, curveOf, EVENTS, loadTuning,
  type DemoAction, type DragPayload, type NativeInfo, type NativeReport, type RegionDiff, type StateSnapshot, type Tuning,
} from "./collapseShared";

const query = new URLSearchParams(location.search);
const automatic = query.has("auto");
const native = isTauri();
const GAP = 6; // Sidebar divider width
const MAIN_MIN = { w: 420, h: 480 };
const START = { mainWidth: 560, height: 652 };
/**
 * Offscreen left reserve of the WebKit viewport (px), like the product's fixed canvas (PopupCanvas.swift):
 * the window shows the viewport from `canvasV`, so a column that appears on the left (the Sidebar, D5) is laid
 * out and painted offscreen first and then revealed by one native window + canvas commit. The page never
 * re-lays out and nothing jumps. Must be at least the widest Sidebar plus its divider.
 */
const RESERVE = 400;

const tuning = reactive<Tuning>(loadTuning());
const model = ref<InboxState>(emptyState());
/** CSS view. It follows the model through the transition driver, so interrupted transitions can reverse. */
const view = ref<View>("expanded");
/** The Sidebar column exists in the expanded layout (D5). Flipped by the driver, not by the model. */
const layoutSidebar = ref(false);
const phase = ref<"idle" | "collapsing" | "expanding" | "resizing" | "growing">("idle");
/** The small layout finished: its list scrolls inside the small window. */
const settled = ref(false);
const timing = reactive({ ms: 440, ease: "cubic-bezier(0.2,0,0,1)" });
/** Canvas x of the window's left edge (px). Everything on the page is laid out relative to it. */
const canvasV = ref(RESERVE);
const mini = ref({ w: 260, h: 200, navH: 120 });
const pinned = ref(true);
const busy = ref(false);
/** AskHuman is the active app. Rust reports it from the popup window's focus events (`demo-app-inactive` / `demo-app-active`). */
const appActive = ref(true);
const toast = ref("");
const error = ref("");
const closed = ref(false);
const flashed = reactive(new Set<number>());
/** Requests whose arrival bubble has not landed yet: their blue dot stays hidden meanwhile (product behaviour). */
const arriving = reactive(new Set<number>());
const arrivalNotice = ref<InstanceType<typeof InboxArrivalNotice> | null>(null);
/** What the user has put into one question: typed text, a chosen option and (C28) the names of dropped files. */
interface QAnswer { text: string; choice: number | null; files: string[] }
/** Answers by `request:question` (C32). A request has one question unless it carries `questions`. */
const answers = reactive<Record<string, QAnswer>>({});
/** The question each request is on, like the product's `current`: where files dropped without aiming go, and what 上一个 / 下一个 move. */
const currentQ = reactive<Record<number, number>>({});
/**
 * Files are being carried over the window, and what a drop at the pointer would hit right now: a request's row, a question
 * card of the open request (vertical layout), or the open request's body as a whole.
 */
type DropTarget = { id: number; q?: number } | "main";
const dragging = ref(false);
const dropTarget = ref<DropTarget | null>(null);
/** The request a running wait is about to open (its row shows a progress bar), and how long that wait is. */
const dwellId = ref(0);
const dwellMs = ref(0);

const stageEl = ref<HTMLElement>();
const navEl = ref<HTMLElement>();
const capsuleEl = ref<HTMLElement>();
const listeners: UnlistenFn[] = [];
const timers = new Set<ReturnType<typeof setTimeout>>();
const osReduce = window.matchMedia("(prefers-reduced-motion: reduce)");
const reduced = computed(() => tuning.reduceMotion || osReduce.matches);

/** Experiments: how frames reach the window server (see CollapseParams.sync in PopupCollapse.swift). */
let syncMode: string | undefined;
/** Switches for isolating where a frame hitch comes from (scripted video runs only). */
const debug = reactive<{ noMotion: boolean; noAnim: boolean; hidden: boolean; shadow: boolean | undefined }>({ noMotion: false, noAnim: false, hidden: false, shadow: undefined });
let seq = 0;
let generation = 0;
let remeasureTimer: ReturnType<typeof setTimeout> | null = null;
let remembered = { ...START };
let lastReport: { kind: string; report: NativeReport } | null = null;

// ------------------------------------------------------------------ derived view data
const requests = computed(() => model.value.requests);
const active = computed(() => requests.value.find(r => r.id === model.value.active) ?? null);
const hasPrimary = computed(() => requests.value.some(r => !r.secondary));
const groups = computed(() => model.value.groupOrder.map(path => {
  let items = requests.value.filter(r => r.path === path);
  if (tuning.secondaryOrder === "bottom") items = [...items.filter(r => !r.secondary), ...items.filter(r => r.secondary)];
  return { path, name: items[0]?.project ?? "", items };
}).filter(g => g.items.length));
const vars = computed(() => ({
  "--v": `${canvasV.value}px`, "--span": `${layoutSidebar.value ? tuning.sidebarWidth + GAP : 0}px`,
  "--sb-w": `${tuning.sidebarWidth}px`, "--ms": `${timing.ms}ms`, "--ease": timing.ease,
  "--mini-w": `${mini.value.w}px`, "--mini-h": `${mini.value.h}px`, "--mini-nav-h": `${mini.value.navH}px`,
  "--win-radius": `${tuning.radius}px`, "--dwell-ms": `${dwellMs.value}ms`,
}));
const answerKey = (id: number, q: number) => `${id}:${q}`;
/** The answer of one question. Created with the request (`seedRequest`); the fallback only keeps reads safe. */
function answerOf(id: number, q = 0): QAnswer {
  return answers[answerKey(id, q)] ?? { text: "", choice: null, files: [] };
}
function questionsOf(r: CRequest): CQuestion[] {
  return r.questions?.length ? r.questions : [{ text: r.message, options: r.options }];
}
const isMulti = (r: CRequest | null | undefined) => (r?.questions?.length ?? 0) > 1;
function answersOf(r: CRequest): QAnswer[] {
  return questionsOf(r).map((_, q) => answerOf(r.id, q));
}
const canSend = computed(() => {
  const r = active.value;
  if (!r) return false;
  const all = answersOf(r);
  if (r.kind === "permission") return all[0].choice != null;
  return all.some(a => a.choice != null || !!a.text.trim() || a.files.length > 0);
});

/** A typed answer or a chosen option in any question: the "草稿" label. (Dropped files have their own "附件 N" label.) */
function hasDraft(id: number) {
  const r = requests.value.find(x => x.id === id);
  return !!r && answersOf(r).some(a => !!a.text.trim() || a.choice != null);
}
function fileCount(id: number) {
  const r = requests.value.find(x => x.id === id);
  return r ? answersOf(r).reduce((sum, a) => sum + a.files.length, 0) : 0;
}
function setCurrentQ(id: number, q: number) {
  if (currentQ[id] !== q) currentQ[id] = q;
}
function stepQuestion(id: number, delta: number) {
  const r = requests.value.find(x => x.id === id);
  if (!r) return;
  setCurrentQ(id, Math.max(0, Math.min(questionsOf(r).length - 1, (currentQ[id] ?? 0) + delta)));
}
function toggleOption(id: number, q: number, option: number) {
  const a = answerOf(id, q);
  a.choice = a.choice === option ? null : option;
  setCurrentQ(id, q);
}
/** The questions the body shows: all of them in the vertical layout, otherwise only the current one. */
const shownQuestions = computed(() => {
  const r = active.value;
  if (!r) return [];
  const all = questionsOf(r).map((question, qi) => ({ question, qi }));
  if (!isMulti(r) || tuning.questionLayout === "vertical") return all;
  return [all[Math.min(currentQ[r.id] ?? 0, all.length - 1)]];
});
const vertical = computed(() => isMulti(active.value) && tuning.questionLayout === "vertical");
/** What a drop on the open request's body as a whole would do, for the pill over it. */
const mainDropLabel = computed(() => {
  const r = active.value;
  return r && isMulti(r) ? `松手添加到第 ${(currentQ[r.id] ?? 0) + 1} 题` : "松手添加到当前请求";
});
function isCardTarget(q: number) {
  const target = dropTarget.value;
  return typeof target === "object" && target !== null && target.q === q && target.id === active.value?.id;
}
function kindLabel(kind: Kind) {
  return kind === "permission" ? "权限" : kind === "stop" ? "结束确认" : "提问";
}
function later(fn: () => void, ms: number) {
  const timer = setTimeout(() => { timers.delete(timer); fn(); }, ms);
  timers.add(timer);
}
function sleep(ms: number) {
  return new Promise<void>(resolve => later(resolve, ms));
}
/** Wait for rendered frames. Hidden windows never run animation frames, so a timer bounds the wait. */
function paint(frames = 2) {
  return new Promise<void>(resolve => {
    let done = false;
    const finish = () => { if (!done) { done = true; resolve(); } };
    const step = (left: number) => requestAnimationFrame(() => (left <= 1 ? finish() : step(left - 1)));
    step(frames);
    later(finish, 120 * frames);
  });
}
function notify(text: string) {
  toast.value = text;
  later(() => { if (toast.value === text) toast.value = ""; }, 1900);
}

// ------------------------------------------------------------------ native bridge
async function nativeInfo(): Promise<NativeInfo | null> {
  return native ? invoke<NativeInfo>("demo_info") : null;
}
async function nativeAnimate(params: Record<string, unknown>): Promise<NativeReport | null> {
  return native ? invoke<NativeReport>("demo_animate", { params }) : null;
}
async function nativePrepare(left: number, width: number, height: number, visibleLeft = 0) {
  if (native) await invoke("demo_prepare", { left, width, height, visibleLeft });
}
async function nativeFront() {
  if (native) await invoke("demo_front").catch(() => {});
}

// ------------------------------------------------------------------ seeds
function seedRequest(kind: Kind = "ask", other = false, multi = false): CRequest {
  const request = buildRequest(kind, other, multi);
  // Every question starts with an empty answer; the page only reads them afterwards.
  questionsOf(request).forEach((_, q) => { answers[answerKey(request.id, q)] = { text: "", choice: null, files: [] }; });
  currentQ[request.id] = 0;
  return request;
}
function buildRequest(kind: Kind, other: boolean, multi: boolean): CRequest {
  const id = ++seq;
  const first = id === 1;
  if (multi) {
    // C32: a request with three questions. The third has no options, so its card is shorter than the others.
    return {
      id, kind: "ask", project: other ? "Website" : "HumanInLoop", path: other ? "/projects/website" : "/projects/HumanInLoop",
      agent: "Codex", session: "发布流程", title: "发布前的三个确认",
      message: "这条消息里有三道题，每道题的回答和附件互相独立。",
      options: [],
      questions: [
        { text: "先发布到哪个环境？", options: ["预发布环境", "生产环境"] },
        { text: "要不要一起迁移数据？", options: ["需要迁移", "不需要迁移"] },
        { text: "还有什么要补充给发布同学的？", options: [] },
      ],
      secondary: false, visited: false,
    };
  }
  return {
    id, kind, project: other ? "Website" : "HumanInLoop", path: other ? "/projects/website" : "/projects/HumanInLoop",
    agent: kind === "permission" ? "Claude Code" : "Codex",
    session: first ? "统一作答窗口设计" : kind === "permission" ? "验证安装结果" : other ? "首页改版" : "弹窗交互优化",
    title: kind === "permission" ? "允许运行项目测试？" : kind === "stop" ? "任务已完成，下一步做什么？"
      : other ? "选择首页的内容布局" : first ? "确认待答列表的导航方式" : "选择新请求的提醒方式",
    message: kind === "permission" ? "Agent 需要在项目中运行测试，确认改动是否通过。"
      : kind === "stop" ? "修改和验证已完成。你可以继续安排任务，或允许本次任务结束。"
      : first ? "多个 Agent 都在等待你的回答。暂时不想回答的请求可以标为「稍后回答」，所有请求都稍后回答后，窗口会缩成一个小窗，随时点开继续。"
      : "这个请求刚刚到达，当前正在回答的内容保持不动。",
    options: kind === "permission" ? ["批准一次", "拒绝"] : kind === "stop" ? ["继续工作", "结束本次任务"]
      : first ? ["按项目分组，组内按到达时间", "按到达时间平铺"] : other ? ["内容与预览左右排列", "内容纵向排列"]
      : ["高亮边栏条目，并合并提示音", "仅更新待答数量"],
    secondary: false, visited: false,
  };
}

// ------------------------------------------------------------------ animation parameters
function axes() {
  if (tuning.stagger === "together") return { widthStart: 0, widthEnd: 1, heightStart: 0, heightEnd: 1 };
  const lag = (1 - tuning.staggerOverlap) * 0.5;
  return tuning.stagger === "height-first"
    ? { heightStart: 0, heightEnd: 1 - lag, widthStart: lag, widthEnd: 1 }
    : { widthStart: 0, widthEnd: 1 - lag, heightStart: lag, heightEnd: 1 };
}
function scaled(ms: number) {
  return reduced.value ? 0 : ms * tuning.slow;
}
function animationParams(kind: "collapse" | "expand" | "resize", size: { w: number; h: number }, extra: Record<string, unknown> = {}) {
  const collapse = kind !== "expand";
  const durationMs = kind === "resize" ? scaled(240) : scaled(collapse ? tuning.collapseMs : tuning.expandMs);
  const curve = curveOf(collapse ? tuning.collapseCurve : tuning.expandCurve, tuning.custom);
  timing.ms = durationMs;
  timing.ease = cssCurve(curve);
  return {
    toWidth: size.w, toHeight: size.h, durationMs, curve, ...axes(),
    lights: kind === "resize" ? (tuning.lights === "hide" ? "settle" : "keep") : collapse ? (tuning.lights === "hide" ? "hide" : "keep") : "show",
    lightsStart: collapse ? 0 : 0.45, lightsEnd: collapse ? 0.35 : 1,
    sync: syncMode,
    ...(debug.shadow === undefined ? {} : { shadow: debug.shadow }),
    ...extra,
  };
}

/**
 * Size of the small window. Measured from the real DOM in its final small layout; transitions are
 * disabled while measuring and the restoring style recalculation also runs with them disabled, so
 * nothing animates because of the measurement.
 */
function measureMini() {
  const stage = stageEl.value!;
  if (tuning.form === "capsule") {
    const capsule = capsuleEl.value!;
    return { w: Math.ceil(capsule.offsetWidth), h: Math.ceil(capsule.offsetHeight), navH: 0 };
  }
  stage.classList.add("no-motion");
  stage.setAttribute("data-measure", "");
  const nav = navEl.value!;
  const entries = [...nav.querySelectorAll<HTMLElement>(".inbox-entry")];
  const navBox = nav.getBoundingClientRect();
  const navTop = navBox.top - stage.getBoundingClientRect().top;
  const last = entries[Math.min(tuning.rows, entries.length) - 1];
  // Row rects are in content coordinates whether or not the list is already height-limited.
  const navH = Math.ceil((last ? last.getBoundingClientRect().bottom - navBox.top + nav.scrollTop : 0) + 16);
  stage.removeAttribute("data-measure");
  void stage.offsetHeight;
  stage.classList.remove("no-motion");
  return { w: tuning.sidebarWidth, h: Math.ceil(navTop + navH), navH };
}

// ------------------------------------------------------------------ transition driver
/** Show a driver error in the window and keep it in the jsonl log (the prototype has no console). */
function fail(e: unknown) {
  error.value = String(e);
  if (native) void invoke("demo_record", { value: { label: "stage-error", detail: String(e) }, finished: false }).catch(() => {});
}

interface Pending { kind: string; from: string; to: string; wantedMs: number }

/**
 * Rendering opportunities the page itself gets while a transition runs (requestAnimationFrame). The native frame
 * statistics only describe the window; a late page frame shows as content lagging behind the moving window edge.
 */
interface PageFrames { frames: number; maxGapMs: number; slowFrames: number }
let pageProbe: { stop(): PageFrames } | null = null;
function startPageProbe() {
  pageProbe?.stop();
  let last = performance.now(), frames = 0, maxGap = 0, slow = 0, running = true;
  const tick = (now: number) => {
    if (!running) return;
    if (frames > 0) {
      maxGap = Math.max(maxGap, now - last);
      if (now - last > 25) slow++;
    }
    last = now;
    frames++;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
  const probe = { stop: () => { running = false; return { frames, maxGapMs: maxGap, slowFrames: slow }; } };
  pageProbe = probe;
}

async function finishTransition(gen: number, pending: Pending, report: NativeReport | null) {
  if (gen !== generation || !report || report.cancelled) return false;
  lastReport = { kind: pending.kind, report };
  const info = await nativeInfo();
  const payload = {
    kind: pending.kind, wantedMs: pending.wantedMs, frames: report.frames, maxGapMs: report.maxGapMs,
    p95GapMs: report.p95GapMs, meanGapMs: report.meanGapMs, dropped: report.dropped, wallMs: report.wallMs,
    maxErrorPx: report.maxErrorPx, refreshHz: report.refreshHz, cancelled: false, from: pending.from, to: pending.to,
    page: pageProbe?.stop() ?? null,
  };
  pageProbe = null;
  if (native) {
    void emit(EVENTS.report, payload);
    void invoke("demo_record", { value: { label: "transition", ...payload, report, info }, finished: false });
  }
  return true;
}

/** Collapse to the small window, or resize it when it is already small. */
async function runCollapse(gen: number, before: NativeInfo | null) {
  const wasExpanded = view.value === "expanded";
  cancelAllArrivals();
  // Only a settled expanded window tells the size to restore. Reversing an in-flight expand (or a
  // Sidebar growth) must keep the value that expand was heading for, not the partial frame.
  if (wasExpanded && phase.value === "idle" && before) {
    remembered = { mainWidth: before.width - (layoutSidebar.value ? tuning.sidebarWidth + GAP : 0), height: before.height };
  }
  // Pin the viewport at its current size so the page never reflows while the window shrinks.
  if (before && !(before.canvasInstalled && !before.canvasResponsive)) {
    await nativePrepare(canvasV.value, before.viewW, before.viewH);
    if (gen !== generation) return;
  }
  const size = measureMini();
  const kind = wasExpanded ? "collapse" : "resize";
  const params = animationParams(kind, size, { resizable: false });
  settled.value = false;
  mini.value = size;
  phase.value = wasExpanded ? "collapsing" : "resizing";
  const run = nativeAnimate(params);
  view.value = "collapsed";
  const report = await run;
  if (!native) await sleep(params.durationMs);
  if (!(await finishTransition(gen, { kind, from: wasExpanded ? "expanded" : "collapsed", to: "collapsed", wantedMs: params.durationMs }, report)) && native) return;
  if (gen !== generation) return;
  settled.value = true;
  phase.value = "idle";
}

/** Expand from the small window back to the remembered size (re-anchored top-left, kept on screen). */
async function runExpand(gen: number, before: NativeInfo | null) {
  const sidebar = sidebarVisible(model.value);
  const extra = sidebar ? tuning.sidebarWidth + GAP : 0;
  const size = { w: remembered.mainWidth + extra, h: remembered.height };
  settled.value = false;
  layoutSidebar.value = sidebar;
  // The full layer is still invisible: pin the viewport at the expanded size before the window grows.
  if (before) {
    const viewW = canvasV.value + size.w;
    const same = before.canvasInstalled && !before.canvasResponsive
      && Math.abs(before.viewW - viewW) < 0.5 && Math.abs(before.viewH - size.h) < 0.5;
    if (!same) {
      await nativePrepare(canvasV.value, viewW, size.h);
      if (gen !== generation) return;
    }
  }
  const params = animationParams("expand", size, {
    resizable: true, resume: true, minWidth: MAIN_MIN.w + extra, minHeight: MAIN_MIN.h,
  });
  phase.value = "expanding";
  const run = nativeAnimate(params);
  view.value = "expanded";
  const report = await run;
  if (!native) await sleep(params.durationMs);
  if (!(await finishTransition(gen, { kind: "expand", from: "collapsed", to: "expanded", wantedMs: params.durationMs }, report)) && native) return;
  if (gen !== generation) return;
  phase.value = "idle";
}

async function drive(target: View) {
  const gen = ++generation;
  startPageProbe();
  const before = await nativeInfo();
  if (gen !== generation) return;
  try {
    if (target === "collapsed") await runCollapse(gen, before);
    else await runExpand(gen, before);
  } catch (e) {
    fail(e);
  }
}

/** Test hook (collapseAuto): awaited at the two protocol steps of a Sidebar growth. */
let growHook: ((step: "layout" | "commit") => Promise<void>) | null = null;

/**
 * D5: the Sidebar appears next to a body that must not move — the window grows to the left. Same protocol as
 * the product (PopupInboxView.geometry): the viewport already has an offscreen left reserve, so
 *  1. the page switches to the final layout while the new column is still offscreen (nothing visible changes;
 *     the viewport keeps its size, so nothing re-lays out either), then
 *  2. one native transaction grows the window to the left and moves the canvas by the same amount, which
 *     reveals a column that is already painted.
 */
async function growSidebar() {
  // A window that is still animating has no final size to grow from.
  for (let i = 0; i < 200 && phase.value !== "idle"; i++) await sleep(16);
  const gen = ++generation;
  const before = await nativeInfo();
  if (gen !== generation) return;
  const extra = tuning.sidebarWidth + GAP;
  phase.value = "growing";
  try {
    if (before) {
      const v0 = canvasV.value;
      // Freeze the viewport exactly as it is (same size, same origin); only the native frame changes below.
      await nativePrepare(v0, before.viewW, before.viewH);
      if (gen !== generation) return;
      canvasV.value = v0 - extra;
      layoutSidebar.value = true;
      await nextTick();
      await paint(2);
      if (growHook) await growHook("layout");
      await nativeAnimate({
        toWidth: before.width + extra, toHeight: before.height, toDX: -extra,
        canvasX0: -v0, canvasX1: -(v0 - extra), durationMs: 0,
        resume: true, minWidth: MAIN_MIN.w + extra, minHeight: MAIN_MIN.h, resizable: true,
      });
      if (growHook) await growHook("commit");
    } else {
      layoutSidebar.value = true;
    }
  } catch (e) {
    fail(e);
  }
  if (gen === generation) phase.value = "idle";
}

// ------------------------------------------------------------------ inbox actions
/** Outcomes whose transition has not finished yet (a click handler starts one before `phase` changes). */
let pendingOutcomes = 0;
function snapshot(): StateSnapshot {
  return {
    view: view.value, phase: phase.value === "idle" && pendingOutcomes > 0 ? "pending" : phase.value, active: model.value.active, pinned: pinned.value, closed: closed.value,
    requests: requests.value.map(r => ({ id: r.id, title: r.title, secondary: r.secondary, visited: r.visited })),
  };
}
function publish() {
  if (native) void emit(EVENTS.state, snapshot());
}

async function endRound() {
  closed.value = true;
  publish();
  if (native) {
    const gen = generation;
    await sleep(automatic ? 50 : 450);
    // A request that arrived during the short grace period already started the next round.
    if (gen !== generation || !closed.value) return;
    await invoke("demo_end_round");
  }
}

/** Apply a model outcome and run the transition it asks for. Resolves when that transition settles. */
async function apply(out: Outcome, options: { front?: boolean } = {}): Promise<void> {
  pendingOutcomes++;
  try {
    await applyOutcome(out, options);
  } finally {
    pendingOutcomes--;
    publish();
  }
}
async function applyOutcome(out: Outcome, options: { front?: boolean }): Promise<void> {
  model.value = out.state;
  publish();
  if (out.closed) return endRound();
  if (options.front) void nativeFront();
  switch (out.effect) {
    case "collapse":
    case "mini-resize":
      await drive("collapsed");
      break;
    case "expand":
      await drive("expanded");
      break;
    case "sidebar":
      await growSidebar();
      break;
    default:
      break;
  }
  publish();
}

function flash(id: number, ms = arrivalTiming.land) {
  flashed.add(id);
  later(() => flashed.delete(id), ms);
}

// ------------------------------------------------------------------ arrival bubble (product component)
function cancelArrival(id: number) {
  arriving.delete(id);
  flashed.delete(id);
  arrivalNotice.value?.cancel(String(id));
}
function cancelAllArrivals() {
  arriving.clear();
  flashed.clear();
  arrivalNotice.value?.clear();
}
function arrivalSettled(key: string, reason: ArrivalEnd) {
  const id = Number(key);
  arriving.delete(id);
  if (reason === "landed" && requests.value.some(r => r.id === id)) flash(id);
}
/** Same geometry as PopupInboxView.measureArrival: from the centre of the window into the row's blue dot. */
function measureArrival(key: string): ArrivalGeometry | null {
  const id = Number(key);
  const stage = stageEl.value;
  if (!stage || !arriving.has(id) || !requests.value.some(r => r.id === id)) return null;
  const row = stage.querySelector<HTMLElement>(`[data-inbox-row="${id}"]`);
  const nav = navEl.value?.getBoundingClientRect();
  const dot = row?.querySelector(".inbox-dot")?.getBoundingClientRect();
  const main = stage.querySelector(".pane-main")?.getBoundingClientRect();
  const side = stage.querySelector(".pane-sidebar")?.getBoundingClientRect();
  if (!nav || !dot || !main || !side || !dot.width || !main.width) return null;
  const x = dot.left + dot.width / 2, y = dot.top + dot.height / 2;
  // A row scrolled out of the list ends the flight (the unread cue stays), as in the product.
  if (x - 5 < nav.left || x + 5 > nav.right || y - 5 < nav.top || y + 5 > nav.bottom) return null;
  const bounds = stage.getBoundingClientRect();
  return {
    origin: { x: (side.left + main.right) / 2 - bounds.left, y: (main.top + main.bottom) / 2 - bounds.top - 10 },
    target: { x: x - bounds.left, y: y - bounds.top },
  };
}
async function playArrival(id: number) {
  if (!arriving.has(id)) return;
  await nextTick();
  const row = stageEl.value?.querySelector<HTMLElement>(`[data-inbox-row="${id}"]`);
  const nav = navEl.value;
  if (row && nav) {
    // Only the list scrolls to the new row; the answer area must not move.
    const r = row.getBoundingClientRect(), n = nav.getBoundingClientRect();
    if (r.top < n.top) nav.scrollTop += r.top - n.top;
    else if (r.bottom > n.bottom) nav.scrollTop += r.bottom - n.bottom;
  }
  if (!arrivalNotice.value || view.value !== "expanded") { cancelArrival(id); return; }
  void arrivalNotice.value.play(String(id));
}

async function arriveRequest(kind: Kind = "ask", other = false, multi = false) {
  // The previous round is over (queue empty, window hidden): the next request is an ordinary first popup again.
  if (closed.value) return startRound(1, kind, other, multi);
  const request = seedRequest(kind, other, multi);
  const first = model.value.requests.length === 0;
  const bubble = !first && tuning.arrivalBubble && !reduced.value;
  if (bubble) arriving.add(request.id);
  else if (!first) flash(request.id);
  await apply(arrive(model.value, request, { showNew: tuning.arrivalBody === "new" }), { front: !first });
  if (bubble) await playArrival(request.id);
}
async function browseRequest(id: number, front = false) {
  if (busy.value) return;
  cancelArrival(id);
  await apply(browse(model.value, id), { front });
}
async function toggleFlag(id: number) {
  const request = requests.value.find(r => r.id === id);
  if (!request || busy.value) return;
  await apply(request.secondary ? unmark(model.value, id) : markSecondary(model.value, id));
}
async function putAwayActive() {
  if (busy.value) return;
  const before = model.value.active;
  await apply(putAway(model.value));
  if (model.value.view === "expanded" && model.value.active !== before) notify("已设为稍后回答，继续下一条");
}
async function expandDefault() {
  const id = defaultOpenTarget(model.value);
  if (id !== null) await apply(browse(model.value, id));
}
/**
 * `remote`: the request was ended by a channel, not by this window. The user only counts as engaged (and is taken to
 * the next "稍后答" request when no primary one is left, C22) if AskHuman is the active app.
 */
async function finishRequest(id: number, note: string, remote = false) {
  if (!requests.value.some(r => r.id === id)) return;
  cancelArrival(id);
  await apply(finish(model.value, id, { engaged: !remote || appActive.value }));
  if (!note || !model.value.requests.length || model.value.view !== "expanded") return;
  notify(`${note}${hasPrimary.value ? "，继续下一条" : "，转到稍后答的请求"}`);
}
/**
 * Shrink the expanded window back to the small window without marking anything. Only a peek (nothing but "稍后答"
 * requests) can shrink; a primary request keeps the window open. Marks, drafts and attachments stay untouched.
 * Callers: the shrink icon (C27), leaving AskHuman (C23) and a drag that gave up (C28).
 */
async function shrinkWindow() {
  if (closed.value) return;
  const out = collapseWindow(model.value);
  if (out.effect) await apply(out);
}
/**
 * C23: the active window moved outside AskHuman while the expanded window shows only "稍后答" requests (a peek).
 * Not while files are being dragged over the window: that user is working with it.
 */
async function collapseOnLeave() {
  if (!tuning.autoCollapseOnLeave || dragging.value) return;
  await shrinkWindow();
}
function appInactive() {
  appActive.value = false;
  void collapseOnLeave();
}

// ------------------------------------------------------------------ files dragged over the window (C28, C31, C32)
// Like dragging onto folders in Finder ("spring-loading"): one drag gesture must be able to reach any pending request and
// any of its questions. Resting over the small window opens it (onto the row under the pointer, otherwise onto the default
// request); resting on another Sidebar row switches the body to that request; near the top or bottom edge of the Sidebar
// list or the body the pointer scrolls it, so every row and every question card can be reached; and a drop attaches the
// files to whatever is under the pointer at that moment, without waiting for any of that. Positions come from Tauri's
// native drag-drop events (the product's mechanism, usePopupCore.ts), not from DOM drag events.
let openedByDrag = false;
let springTimer: ReturnType<typeof setTimeout> | null = null;
let dwell: { id: number; timer: ReturnType<typeof setTimeout> } | null = null;
/** Page coordinates of the pointer at the last event: the edge scrolling and re-aiming after a layout change use it. */
let lastPointer: { x: number; y: number } | null = null;
/** Something opened or switched under a pointer that did not move: the next wait only starts once it has moved. */
let holdUntil: { x: number; y: number } | null = null;

const animating = () => phase.value !== "idle" || pendingOutcomes > 0;

function clearDwell() {
  if (dwell) clearTimeout(dwell.timer);
  dwell = null;
  dwellId.value = 0;
}
function clearSpringBack() {
  if (springTimer) clearTimeout(springTimer);
  springTimer = null;
}
/**
 * wry measures y from the top of the webview *frame* (`frame.height - draggingLocation.y`). While the viewport is
 * pinned larger than the window — the small window, or a window that is still moving — that frame sticks out below
 * the window, so y comes out too large by the difference. A settled expanded window has a responsive viewport and no
 * difference. (x is relative to the window and needs only the canvas origin.)
 */
async function pinnedExcess(): Promise<number> {
  const settledNow = view.value === "expanded" && phase.value === "idle" && pendingOutcomes === 0;
  if (!native || settledNow) return 0;
  const info = await nativeInfo();
  return info ? Math.max(0, info.viewH - info.height) : 0;
}
function targetAt(x: number, y: number): DropTarget | null {
  const element = document.elementFromPoint(x, y);
  const row = element?.closest<HTMLElement>(".inbox-entry")?.querySelector<HTMLElement>("[data-inbox-row]");
  if (row) return { id: Number(row.dataset.inboxRow) };
  if (!element?.closest(".pane-main")) return null;
  // C32: in the vertical layout the question card under the pointer is the target; elsewhere on the body, the body as a whole.
  const card = element.closest<HTMLElement>(".pc-question.card");
  const id = active.value?.id;
  return card && id !== undefined ? { id, q: Number(card.dataset.qIndex) } : "main";
}
/**
 * Aim at what is under the pointer and, unless that is already what is open, start the wait that opens it: the small
 * window opens onto a row's request (anywhere else on it: the default one), an open window switches to another row's.
 */
async function pointerOver(x: number, y: number) {
  lastPointer = { x, y };
  const target = targetAt(x, y);
  dropTarget.value = target;
  if (holdUntil) {
    if (Math.hypot(x - holdUntil.x, y - holdUntil.y) < 4) return;
    holdUntil = null;
  }
  const collapsed = model.value.view === "collapsed";
  const row = target && target !== "main" && target.q === undefined ? target.id : null;
  const wanted = collapsed ? row ?? defaultOpenTarget(model.value) : row !== null && row !== model.value.active ? row : null;
  // Nothing starts while the window is still moving: the layout under the pointer is not the final one.
  if (wanted === null || animating()) {
    clearDwell();
    return;
  }
  if (dwell?.id === wanted) return;
  clearDwell();
  const ms = collapsed ? tuning.dragExpandMs : tuning.dragSwitchMs;
  if (ms <= 0) return openByDrag(wanted);
  dwellId.value = wanted;
  dwellMs.value = ms;
  dwell = { id: wanted, timer: setTimeout(() => { dwell = null; dwellId.value = 0; void openByDrag(wanted); }, ms) };
}
/** A wait ran out (or there is none): open the small window onto `id`, or switch the open window's body to it. */
async function openByDrag(id: number) {
  if (closed.value || !dragging.value) return;
  if (model.value.view === "collapsed") openedByDrag = true;
  // The layout is about to move under a pointer that will not: aim again afterwards, and wait for it to move.
  holdUntil = lastPointer ? { ...lastPointer } : null;
  await browseRequest(id);
  traceDrag("opened", { id });
  if (lastPointer) dropTarget.value = targetAt(lastPointer.x, lastPointer.y);
}
function dragLeft() {
  dragging.value = false;
  dropTarget.value = null;
  lastPointer = null;
  holdUntil = null;
  clearDwell();
  stopEdgeScroll();
  if (!openedByDrag) return;
  if (!tuning.dragSpringBack) { openedByDrag = false; return; }
  // The drag opened the window and gave up: close it again, unless it comes back within the grace period.
  clearSpringBack();
  springTimer = setTimeout(() => {
    springTimer = null;
    if (dragging.value) return;
    openedByDrag = false;
    void shrinkWindow();
  }, 600);
}
async function dropFiles(paths: string[], x: number, y: number) {
  const hit = targetAt(x, y) ?? dropTarget.value;
  dragging.value = false;
  dropTarget.value = null;
  lastPointer = null;
  holdUntil = null;
  clearDwell();
  clearSpringBack();
  stopEdgeScroll();
  openedByDrag = false;
  // Not on a row or a card: the open request (for the small window: the one it would open).
  const fallback = model.value.view === "collapsed" ? defaultOpenTarget(model.value) : model.value.active;
  const id = hit && hit !== "main" ? hit.id : fallback ?? defaultOpenTarget(model.value);
  const request = requests.value.find(r => r.id === id);
  if (!request || !paths.length) return;
  // C32: a card names its question; anything else goes to the request's current one (the first until it has been looked at).
  const q = hit && hit !== "main" && hit.q !== undefined ? hit.q : currentQ[request.id] ?? 0;
  const names = paths.map(path => path.split(/[\\/]/).pop() || path);
  answerOf(request.id, q).files.push(...names);
  setCurrentQ(request.id, q);
  notify(`已添加 ${names.length} 个文件到「${request.title}」${isMulti(request) ? `的第 ${q + 1} 题` : ""}`);
  // Show the request that received them (expands a small window; an already open one stays put).
  await browseRequest(request.id);
  // Dropping is working with the window: take the focus so that leaving AskHuman afterwards shrinks a peek (C23).
  if (tuning.dropActivates && native && !appActive.value) await invoke("demo_ready").catch(() => {});
}

// Edge scrolling (C31): the Sidebar list and the body scroll while the pointer rests near their top or bottom edge, faster
// the closer it gets, so that any row and any question card can be reached without letting go.
const EDGE = 56;
const EDGE_SPEED = 900;
let edgeFrame = 0;
let edgeLast = 0;
let edgeEl: HTMLElement | null = null;
let edgePos = 0;
/** The list or body under the pointer and the signed speed (px/s) it should scroll at; null when it should stay put. */
function edgePush(x: number, y: number): { el: HTMLElement; speed: number } | null {
  const stage = stageEl.value;
  if (!stage) return null;
  const side = stage.querySelector<HTMLElement>(".pane-sidebar")?.getBoundingClientRect();
  const inSide = !!side && side.width > 0 && x >= side.left && x < side.right;
  const el = inSide ? navEl.value : stage.querySelector<HTMLElement>(".pc-bodies .pc-body");
  if (!el) return null;
  const box = el.getBoundingClientRect();
  // Anything above the top edge zone (the heading, the navigation bar) pushes as hard as the edge itself.
  let push = 0;
  if (y < box.top + EDGE) push = -Math.min(1, (box.top + EDGE - y) / EDGE);
  else if (y > box.bottom - EDGE) push = Math.min(1, (y - (box.bottom - EDGE)) / EDGE);
  if (!push) return null;
  const room = push < 0 ? el.scrollTop > 0 : el.scrollTop < el.scrollHeight - el.clientHeight - 0.5;
  return room ? { el, speed: Math.sign(push) * EDGE_SPEED * push * push } : null;
}
function edgeTick(now: number) {
  edgeFrame = 0;
  if (!dragging.value || !lastPointer) {
    edgeEl = null;
    return;
  }
  const dt = edgeLast ? Math.min(now - edgeLast, 50) : 16;
  edgeLast = now;
  const push = edgePush(lastPointer.x, lastPointer.y);
  if (push) {
    // Fractions of a pixel add up here: the element's own scrollTop would round them away at slow speeds.
    if (push.el !== edgeEl || Math.abs(push.el.scrollTop - edgePos) > 2) {
      edgeEl = push.el;
      edgePos = push.el.scrollTop;
    }
    edgePos += (push.speed * dt) / 1000;
    push.el.scrollTop = edgePos;
    // Whatever slid under the pointer is the target now.
    void pointerOver(lastPointer.x, lastPointer.y);
  } else {
    edgeEl = null;
  }
  edgeFrame = requestAnimationFrame(edgeTick);
}
function startEdgeScroll() {
  if (edgeFrame) return;
  edgeLast = 0;
  edgeFrame = requestAnimationFrame(edgeTick);
}
function stopEdgeScroll() {
  if (edgeFrame) cancelAnimationFrame(edgeFrame);
  edgeFrame = 0;
  edgeEl = null;
}

/**
 * Native drag events arrive in bursts: `enter`, `over` a millisecond later, then another every few milliseconds. Handled
 * concurrently, a later `over` overtook the `enter` that was about to open the small window and the window stayed shut.
 * So the events are handled one at a time, in order; pointer moves that pile up behind a slow step (the window opening)
 * collapse into the newest one, since only the latest position matters.
 */
let dragChain: Promise<void> = Promise.resolve();
let queuedOver: DragPayload | null = null;
function onDragDrop(payload: DragPayload): Promise<void> {
  if (closed.value) return Promise.resolve();
  if (payload.type === "over") {
    const waiting = queuedOver !== null;
    queuedOver = payload;
    if (waiting) return dragChain;
    dragChain = dragChain
      .then(() => {
        const latest = queuedOver;
        queuedOver = null;
        return latest ? handleDrag(latest) : undefined;
      })
      .catch(fail);
    return dragChain;
  }
  // A pointer move queued before this event describes a situation that is already over.
  queuedOver = null;
  dragChain = dragChain.then(() => handleDrag(payload)).catch(fail);
  return dragChain;
}
/** What the page does with the drag, to read back from the log after trying a real drag (interactive runs only). */
let tracedAt = 0;
let tracedTarget = "";
function traceDrag(kind: string, detail: Record<string, unknown> = {}) {
  if (!native || automatic) return;
  const now = performance.now();
  const hit = dropTarget.value;
  const target = hit === null ? "-" : hit === "main" ? "main" : hit.q === undefined ? String(hit.id) : `${hit.id}.${hit.q}`;
  // Pointer moves come every few ms: log them only when they hit something else, and at a slow pace otherwise.
  if (kind === "over" && target === tracedTarget && now - tracedAt < 400) return;
  tracedAt = now;
  tracedTarget = target;
  void invoke("demo_record", {
    value: { label: "drag", kind, view: view.value, phase: phase.value, target, dragging: dragging.value, wait: dwellId.value, ...detail },
    finished: false,
  }).catch(() => {});
}
async function handleDrag(payload: DragPayload) {
  if (closed.value) return;
  if (payload.type === "leave") {
    dragLeft();
    traceDrag("leave");
    return;
  }
  const excess = await pinnedExcess();
  const x = payload.position.x + canvasV.value, y = payload.position.y - excess;
  const geometry = { raw: [payload.position.x, payload.position.y], x: Math.round(x), y: Math.round(y), excess: Math.round(excess), canvasV: Math.round(canvasV.value) };
  if (payload.type === "drop") {
    const before = { ...geometry, files: payload.paths.length };
    await dropFiles(payload.paths, x, y);
    traceDrag("drop", { ...before, active: model.value.active });
    return;
  }
  // `enter` normally starts a drag, but the first move does as well should the `enter` ever be missed.
  if (!dragging.value) {
    dragging.value = true;
    clearSpringBack();
  }
  await pointerOver(x, y);
  traceDrag(payload.type, geometry);
  startEdgeScroll();
}
function removeAttachment(id: number, q: number, index: number) {
  answerOf(id, q).files.splice(index, 1);
}
async function sendActive() {
  const request = active.value;
  if (!request || busy.value || !canSend.value) return;
  busy.value = true;
  await sleep(automatic ? 20 : 180);
  busy.value = false;
  await finishRequest(request.id, "已发送");
}
async function cancelActive() {
  const request = active.value;
  if (!request || busy.value) return;
  await finishRequest(request.id, "已取消当前请求");
}
function togglePin() {
  void setPin(!pinned.value);
}
async function setPin(on: boolean) {
  pinned.value = on;
  if (native) await invoke("demo_pin", { on });
  publish();
}

async function startRound(count: number, firstKind: Kind = "ask", firstOther = false, firstMulti = false) {
  generation++;
  seq = 0;
  for (const key of Object.keys(answers)) delete answers[key];
  for (const key of Object.keys(currentQ)) delete currentQ[Number(key)];
  dragging.value = false;
  dropTarget.value = null;
  lastPointer = null;
  holdUntil = null;
  openedByDrag = false;
  clearDwell();
  clearSpringBack();
  stopEdgeScroll();
  cancelAllArrivals();
  closed.value = false;
  busy.value = false;
  error.value = "";
  let state = emptyState();
  for (let i = 0; i < count; i++) {
    const kind: Kind = i === 0 ? firstKind : i === 1 ? "permission" : "ask";
    state = arrive(state, seedRequest(kind, i === 0 ? firstOther : i === 2, i === 0 && firstMulti)).state;
  }
  model.value = state;
  view.value = "expanded";
  settled.value = false;
  phase.value = "idle";
  layoutSidebar.value = sidebarVisible(state);
  remembered = { ...START };
  const extra = layoutSidebar.value ? tuning.sidebarWidth + GAP : 0;
  // The window always shows the viewport from `canvasV`: main body at RESERVE, Sidebar (if any) to its left.
  canvasV.value = RESERVE - extra;
  const info = await nativeInfo();
  const size = { w: START.mainWidth + extra, h: START.height };
  if (info && (Math.abs(info.width - size.w) > 0.5 || Math.abs(info.height - size.h) > 0.5
    || !info.canvasInstalled || !info.canvasResponsive || Math.abs(info.canvasX + canvasV.value) > 0.5
    || info.lightsHidden || !info.resizable)) {
    await nativePrepare(canvasV.value, canvasV.value + size.w, size.h);
    await nativeAnimate({
      toWidth: size.w, toHeight: size.h, durationMs: 0, canvasX0: -canvasV.value, canvasX1: -canvasV.value,
      resume: true, lights: "show", minWidth: MAIN_MIN.w + extra, minHeight: MAIN_MIN.h, resizable: true,
    });
  }
  await nextTick();
  await paint(2);
  if (native) {
    await invoke("demo_pin", { on: pinned.value });
    await invoke("demo_ready");
    // The window was just shown and focused by us; the focus event follows asynchronously.
    appActive.value = true;
    await warmUp(extra);
  }
  publish();
}

/**
 * The very first display-link run in a process stalls for a few frames (display link attach, first
 * material re-composite). Run a no-change animation once after the window is shown so the first
 * real collapse is as smooth as the later ones.
 */
let warmed = false;
async function warmUp(extra: number) {
  if (warmed) return;
  warmed = true;
  const info = await nativeInfo();
  if (!info) return;
  await nativeAnimate({
    toWidth: info.width, toHeight: info.height, durationMs: 120, curve: [0.2, 0, 0, 1], lights: "keep",
    resume: true, resizable: true, minWidth: MAIN_MIN.w + extra, minHeight: MAIN_MIN.h,
  });
}

async function perform(action: DemoAction) {
  switch (action.type) {
    case "arrive": return arriveRequest(action.kind, action.other, action.multi);
    case "batch":
      await arriveRequest("ask");
      await arriveRequest("permission");
      return arriveRequest("ask", true);
    case "delayed":
      notify(`${action.seconds} 秒后到达新提问`);
      later(() => { void arriveRequest("ask"); }, action.seconds * 1000);
      return;
    case "delayedEnd":
      notify(`${action.seconds} 秒后远端终结当前条`);
      later(() => { void finishRequest(model.value.active ?? -1, "当前请求已在飞书回答", true); }, action.seconds * 1000);
      return;
    case "leave": return collapseOnLeave();
    case "shrink": return shrinkWindow();
    case "putAway": return putAwayActive();
    case "markAll": {
      for (const r of requests.value.filter(r => r.id !== model.value.active)) await apply(markSecondary(model.value, r.id));
      return putAwayActive();
    }
    case "unmarkAll": {
      for (const r of requests.value.filter(r => r.secondary)) await apply(unmark(model.value, r.id));
      return;
    }
    case "answer": return sendActive();
    case "endActive": return finishRequest(model.value.active ?? -1, "当前请求已在飞书回答", true);
    case "endLast": {
      const all = orderedRequests(model.value);
      return finishRequest(all[all.length - 1]?.id ?? -1, "", true);
    }
    case "trayFocus": {
      const target = requests.value.find(r => r.id !== model.value.active) ?? active.value;
      if (target) await browseRequest(target.id, true);
      return;
    }
    case "expandDefault": return expandDefault();
    case "reset": return startRound(action.count);
    case "dragScene": {
      await startRound(3, "ask", false, true);
      await sleep(250);
      return perform({ type: "markAll" });
    }
    case "pin": return setPin(action.on);
  }
}

// ------------------------------------------------------------------ small-window interaction
function capsuleDown(event: PointerEvent) {
  if (event.button !== 0) return;
  const element = event.currentTarget as HTMLElement;
  const startX = event.clientX, startY = event.clientY;
  let dragging = false;
  element.setPointerCapture(event.pointerId);
  const stop = () => {
    element.removeEventListener("pointermove", move);
    element.removeEventListener("pointerup", up);
    element.removeEventListener("pointercancel", stop);
  };
  const move = (e: PointerEvent) => {
    if (dragging || Math.hypot(e.clientX - startX, e.clientY - startY) <= 3) return;
    dragging = true;
    stop();
    if (element.hasPointerCapture(event.pointerId)) element.releasePointerCapture(event.pointerId);
    if (native) void getCurrentWindow().startDragging();
  };
  const up = () => { stop(); if (!dragging) void expandDefault(); };
  element.addEventListener("pointermove", move);
  element.addEventListener("pointerup", up);
  element.addEventListener("pointercancel", stop);
}

function keydown(event: KeyboardEvent) {
  if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); void sendActive(); }
}

// ------------------------------------------------------------------ lifecycle
onMounted(async () => {
  window.addEventListener("keydown", keydown);
  if (native) {
    listeners.push(await listen<Partial<Tuning>>(EVENTS.tuning, e => {
      const keys = ["form", "rows", "sidebarWidth", "density"] as const;
      const before = keys.map(key => tuning[key]);
      Object.assign(tuning, e.payload);
      // Anything that changes the small window's content re-measures and resizes it. Slider drags
      // send many events: wait for a short quiet period, then follow the latest value (a resize in
      // flight is interrupted and continues from its current frame).
      const changed = keys.some((key, i) => tuning[key] !== before[i]);
      if (!changed || view.value !== "collapsed") return;
      if (remeasureTimer) clearTimeout(remeasureTimer);
      remeasureTimer = setTimeout(() => {
        remeasureTimer = null;
        if (view.value === "collapsed") void drive("collapsed").then(publish);
      }, 120);
    }));
    listeners.push(await listen<DemoAction>(EVENTS.action, e => { void perform(e.payload); }));
    listeners.push(await listen(EVENTS.hello, () => publish()));
    listeners.push(await listen("demo-app-inactive", appInactive));
    listeners.push(await listen("demo-app-active", () => { appActive.value = true; }));
    listeners.push(await getCurrentWebview().onDragDropEvent(e => { void onDragDrop(e.payload as DragPayload).catch(fail); }));
    listeners.push(await listen<string>("demo-action", e => { if (e.payload === "close") notify("关闭按钮：原型不接关闭确认层"); }));
    void emit(EVENTS.hello);
  }
  await startRound(tuning.initialCount);
  if (automatic) {
    const api: StageApi = {
      perform, info: nativeInfo, snapshot, lastReport: () => lastReport, mini: () => mini.value, tuning,
      sidebarExtra: () => tuning.sidebarWidth + GAP,
      record: async (value, finished) => { if (native) await invoke("demo_record", { value, finished }); },
      canvasV: () => canvasV.value,
      arrivals: () => arriving.size,
      silent: query.has("silent"),
      setAppActive: on => { appActive.value = on; },
      appActive: () => appActive.value,
      drag: onDragDrop,
      attachmentCount: (id, q) => {
        const r = requests.value.find(x => x.id === id);
        if (!r) return 0;
        return q === undefined ? fileCount(id) : answerOf(id, q).files.length;
      },
      currentQuestion: id => currentQ[id] ?? 0,
      deactivate: () => (native ? invoke<boolean>("demo_deactivate") : Promise.resolve(false)),
      activate: async () => { if (native) await invoke("demo_ready"); },
      setGrowHook: hook => { growHook = hook; },
      capture: (label, path) => (native ? invoke<void>("demo_capture", { label, path: path ?? null }) : Promise.resolve()),
      diff: (a, b, region) => invoke<RegionDiff>("demo_diff", { a, b, region }),
      burstStart: (name, maxFrames, pauseMicroseconds = 0, writeAll = true) =>
        invoke<void>("demo_burst_start", { name, maxFrames, pauseMicroseconds, writeAll }),
      setSync: mode => { syncMode = mode; },
      setDebug: flags => { Object.assign(debug, flags); },
      control: (mode, cycles, restMs, sizes) => (native ? invoke<void>("demo_control", { mode, cycles, restMs, sizes }) : Promise.resolve()),
      backdrop: (enable, region) => (native ? invoke<void>("demo_backdrop", { enable, region }) : Promise.resolve()),
      burstStop: async () => {
        const [frames, tears] = await invoke<number[]>("demo_burst_stop");
        return { frames, tears };
      },
      click: selector => {
        const element = stageEl.value?.querySelector<HTMLElement>(selector);
        if (!element || (element instanceof HTMLButtonElement && element.disabled)) return false;
        element.click();
        return true;
      },
      rect: selector => {
        const box = stageEl.value?.querySelector(selector)?.getBoundingClientRect();
        return box ? { left: box.left, top: box.top, width: box.width, height: box.height } : null;
      },
    };
    const auto = query.get("auto");
    void (auto === "burst" ? runBurst(api) : auto === "video" ? runVideo(api) : runScenario(api));
  }
});
onBeforeUnmount(() => {
  if (remeasureTimer) clearTimeout(remeasureTimer);
  clearDwell();
  clearSpringBack();
  stopEdgeScroll();
  window.removeEventListener("keydown", keydown);
  listeners.forEach(off => off());
  timers.forEach(clearTimeout);
});

defineExpose({ perform, nativeInfo, model, view, phase, tuning, snapshot, lastReport: () => lastReport });
</script>

<template>
  <div
    ref="stageEl" class="stage" :class="{ 'no-motion': reduced || debug.noMotion, 'no-anim': debug.noAnim }" :style="[vars, debug.hidden ? { visibility: 'hidden' } : {}]"
    :data-view="view" :data-sidebar="layoutSidebar ? 'on' : 'off'" :data-form="tuning.form"
    :data-salience="tuning.salience" :data-density="tuning.density" :data-lights="tuning.lights"
    :data-rowaction="tuning.rowAction" :data-phase="phase" :data-primary="hasPrimary ? 'yes' : 'no'" :data-settled="settled ? 'yes' : 'no'"
    :data-dragging="dragging ? 'yes' : 'no'" @dragover.prevent @drop.prevent
  >
    <aside class="pane-sidebar" :inert="view === 'expanded' && !layoutSidebar" aria-label="待回答请求">
      <div class="sb-lift">
        <header class="inbox-heading" data-tauri-drag-region>
          <span data-tauri-drag-region>待答 <span class="inbox-count" data-tauri-drag-region>{{ requests.length }}</span></span>
          <button class="mini-expand" aria-label="展开" title="展开" @click="expandDefault">
            <svg viewBox="0 0 24 24"><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" /></svg>
          </button>
          <!-- C27: the way back to the small window, same slot as the expand icon; only while nothing is left to answer (a peek). -->
          <button class="mini-shrink" aria-label="缩回小窗" title="缩回小窗" @click="shrinkWindow">
            <svg viewBox="0 0 24 24"><path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" /></svg>
          </button>
        </header>
        <nav ref="navEl" class="inbox-navigation">
          <section v-for="g in groups" :key="g.path" class="inbox-group">
            <h2 :title="g.path"><span>{{ g.name }}</span><span>{{ g.items.length }}</span></h2>
            <TransitionGroup name="pc-list" tag="div" move-class="pc-move" class="inbox-rows">
              <div v-for="r in g.items" :key="r.id" class="inbox-entry">
                <button
                  class="inbox-row" :data-inbox-row="r.id"
                  :class="{ selected: r.id === model.active && view === 'expanded', secondary: r.secondary, flash: flashed.has(r.id), unread: !r.visited, arriving: arriving.has(r.id), 'drop-target': typeof dropTarget === 'object' && dropTarget?.id === r.id && dropTarget.q === undefined, dwelling: dwellId === r.id }"
                  :aria-current="r.id === model.active && view === 'expanded' ? 'true' : undefined" :title="r.title" @click="browseRequest(r.id)"
                >
                  <UnreadRipple :active="!r.visited && !flashed.has(r.id) && !arriving.has(r.id) && !reduced && !debug.noAnim" />
                  <span class="inbox-dot" :class="{ unread: !r.visited }" :aria-label="!r.visited ? '未查看' : undefined"></span>
                  <span class="inbox-row-content">
                    <strong><span v-if="r.kind !== 'ask'" class="inbox-kind">{{ kindLabel(r.kind) }}</span>{{ r.title }}</strong>
                    <span class="inbox-row-meta">
                      <span class="inbox-row-agent">{{ r.agent }}</span>
                      <span class="inbox-source-separator" aria-hidden="true">/</span>
                      <span class="inbox-session"><SessionIcon /><span class="inbox-session-title">{{ r.session }}</span></span>
                      <em v-if="fileCount(r.id)">附件 {{ fileCount(r.id) }}</em>
                      <em v-if="hasDraft(r.id)">草稿</em>
                      <em v-if="r.secondary" class="inbox-put-away">稍后答</em>
                    </span>
                  </span>
                </button>
                <button
                  class="row-flag" :class="[`flag-${tuning.rowAction}`, { 'is-secondary': r.secondary }]" :aria-label="r.secondary ? '取消稍后答' : '稍后回答'"
                  :title="r.secondary ? '取消稍后答：恢复为待答' : '稍后回答：先不回答这条'" @click.stop="toggleFlag(r.id)"
                >
                  <!-- Archive box: one slot, three looks (hover affordance, "稍后答" label, hover "取消稍后答"). -->
                  <span v-if="tuning.rowAction === 'archive'" class="fa-stack">
                    <span class="fa-put"><svg viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="M10 12h4" /></svg></span>
                    <span class="fa-state"><svg viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="M10 12h4" /></svg>稍后答</span>
                    <span class="fa-undo"><svg viewBox="0 0 24 24"><rect x="2" y="3" width="20" height="5" rx="1" /><path d="M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8" /><path d="m9 15 3-3 3 3" /><path d="M12 12v9" /></svg>取消稍后答</span>
                  </span>
                  <template v-else-if="tuning.rowAction === 'text'">{{ r.secondary ? "取消稍后答" : "稍后回答" }}</template>
                  <svg v-else-if="tuning.rowAction === 'clock'" viewBox="0 0 24 24">
                    <template v-if="!r.secondary"><circle cx="12" cy="12" r="9" /><path d="M12 7v5l3 2" /></template>
                    <template v-else><path d="M9 14 4 9l5-5" /><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11" /></template>
                  </svg>
                  <svg v-else viewBox="0 0 24 24">
                    <template v-if="!r.secondary"><path d="m7 6 5 5 5-5" /><path d="m7 13 5 5 5-5" /></template>
                    <template v-else><path d="m17 11-5-5-5 5" /><path d="m17 18-5-5-5 5" /></template>
                  </svg>
                </button>
              </div>
            </TransitionGroup>
          </section>
        </nav>
      </div>
    </aside>
    <div class="inbox-divider" aria-hidden="true"></div>

    <main class="pane-main" :class="{ 'drop-target': dropTarget === 'main' }" :data-drop-label="mainDropLabel" :inert="view !== 'expanded'">
      <header class="pc-navbar" data-tauri-drag-region>
        <span class="pc-origin" data-tauri-drag-region><span class="pc-dot"></span>{{ active?.agent }} <span class="pc-chip">{{ active?.project }}</span></span>
        <span class="pc-actions">
          <!-- C27: a window without a Sidebar (single request) has no Sidebar header, so a peek offers the icon here instead. -->
          <button class="pc-icon pc-shrink" aria-label="缩回小窗" title="缩回小窗" @click="shrinkWindow">
            <svg viewBox="0 0 24 24"><path d="M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7" /></svg>
          </button>
          <button class="pc-icon" :aria-pressed="pinned" :title="pinned ? '取消置顶' : '置顶'" @click="togglePin">
            <svg viewBox="0 0 24 24"><path d="M12 17v5M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z" /></svg>
          </button>
        </span>
      </header>
      <p v-if="toast" class="pc-toast" role="status">{{ toast }}</p>
      <p v-if="error" class="pc-error" role="alert">{{ error }}</p>
      <div class="pc-bodies">
        <Transition name="pc-swap" mode="out-in">
          <article v-if="active" :key="active.id" class="pc-body" :data-layout="isMulti(active) ? tuning.questionLayout : 'single'">
            <p class="pc-session">{{ active.session }} · {{ kindLabel(active.kind) }}<template v-if="active.secondary"> · 稍后答</template></p>
            <h1>{{ active.title }}</h1>
            <p class="pc-message">{{ active.message }}</p>
            <pre v-if="active.kind === 'permission'" class="pc-command"><code>pnpm test</code></pre>
            <!-- C32: one section per question; a request with several shows the current one (sequential) or all of them as cards (vertical). -->
            <section
              v-for="{ question, qi } in shownQuestions" :key="qi" class="pc-question"
              :class="{ multi: isMulti(active), card: vertical, current: vertical && qi === (currentQ[active.id] ?? 0), 'drop-target': isCardTarget(qi) }"
              :data-q-index="qi" :data-drop-label="`松手添加到第 ${qi + 1} 题`"
              @pointerdown="setCurrentQ(active.id, qi)" @focusin="setCurrentQ(active.id, qi)"
            >
              <template v-if="isMulti(active)">
                <header class="pc-qhead">第 {{ qi + 1 }} / {{ questionsOf(active).length }} 题</header>
                <p class="pc-qtext">{{ question.text }}</p>
              </template>
              <div v-if="question.options.length" class="pc-options">
                <button v-for="(option, i) in question.options" :key="i" class="pc-option" :class="{ chosen: answerOf(active.id, qi).choice === i }" :aria-pressed="answerOf(active.id, qi).choice === i" :disabled="busy" @click="toggleOption(active.id, qi, i)">
                  <span class="pc-option-index">{{ i + 1 }}</span><span>{{ option }}</span>
                </button>
              </div>
              <template v-if="active.kind !== 'permission' || answerOf(active.id, qi).choice === 1">
                <label class="pc-label" :for="`answer-${active.id}-${qi}`">{{ active.kind === 'permission' ? '拒绝原因（可选）' : '你的回答' }}</label>
                <textarea :id="`answer-${active.id}-${qi}`" v-model="answerOf(active.id, qi).text" class="pc-answer" rows="3" :disabled="busy" placeholder="也可以直接写下你的想法…"></textarea>
              </template>
              <ul v-if="answerOf(active.id, qi).files.length" class="pc-attachments" aria-label="附件">
                <li v-for="(name, i) in answerOf(active.id, qi).files" :key="`${name}-${i}`" class="pc-attachment">
                  <svg viewBox="0 0 24 24"><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="9" cy="9" r="2" /><path d="m21 15-3.1-3.1a2 2 0 0 0-2.8 0L6 21" /></svg>
                  <span :title="name">{{ name }}</span>
                  <button :aria-label="`移除 ${name}`" title="移除" @click="removeAttachment(active.id, qi, i)">×</button>
                </li>
              </ul>
            </section>
          </article>
        </Transition>
      </div>
      <footer class="pc-footer">
        <button class="btn" data-act="cancel" :disabled="busy || !active" @click="cancelActive">取消</button>
        <!-- C24: the same slot toggles. A request already marked "稍后答" offers its undo, nothing else. -->
        <button
          v-if="active?.secondary" class="btn" data-act="unmark" :disabled="busy" title="取消稍后答：恢复为待答，窗口保持展开"
          @click="toggleFlag(active.id)"
        >取消稍后答</button>
        <button
          v-else class="btn" data-act="put-away" :disabled="busy || !active"
          title="稍后回答：先不回答这条，窗口留在原地；所有请求都稍后回答后缩成小窗" @click="putAwayActive"
        >稍后回答</button>
        <span class="spacer"></span>
        <template v-if="active && isMulti(active) && !vertical">
          <button class="btn" data-act="prev-q" :disabled="busy || (currentQ[active.id] ?? 0) === 0" @click="stepQuestion(active.id, -1)">上一个</button>
          <button class="btn" data-act="next-q" :disabled="busy || (currentQ[active.id] ?? 0) >= questionsOf(active).length - 1" @click="stepQuestion(active.id, 1)">下一个</button>
        </template>
        <button class="btn btn-primary" data-act="send" :disabled="busy || !canSend" @click="sendActive">{{ busy ? '处理中…' : active?.kind === 'permission' ? '提交决定' : '发送' }}<kbd>⌘↵</kbd></button>
      </footer>
    </main>

    <div
      ref="capsuleEl" class="capsule" role="button" tabindex="0" :aria-label="`展开 ${requests.length} 个待答请求`"
      :inert="!(view === 'collapsed' && tuning.form === 'capsule')" @pointerdown="capsuleDown" @keydown.enter="expandDefault"
    >
      <span class="cap-dot"></span><span class="cap-count">{{ requests.length }} 个待答</span>
      <span class="cap-chevron"><svg viewBox="0 0 24 24"><path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" /></svg></span>
    </div>
    <div class="mini-ring" aria-hidden="true"></div>
    <InboxArrivalNotice ref="arrivalNotice" :measure="measureArrival" @settled="arrivalSettled" />
  </div>
</template>
