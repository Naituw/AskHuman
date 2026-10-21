// Scripted geometry / state checks for the native prototype (`scripts/popup-collapse-demo.sh --auto`).
// The thresholds only cover correctness; frame pacing numbers are recorded for review, not asserted
// (they depend on the machine, the display refresh rate and what else is running).
import {
  DEFAULT_TUNING,
  type DemoAction, type DragPayload, type NativeInfo, type NativeReport, type RegionDiff, type StateSnapshot, type Tuning,
} from "./collapseShared";

export interface StageApi {
  perform(action: DemoAction): Promise<void>;
  info(): Promise<NativeInfo | null>;
  snapshot(): StateSnapshot;
  lastReport(): { kind: string; report: NativeReport } | null;
  mini(): { w: number; h: number; navH: number };
  tuning: Tuning;
  sidebarExtra(): number;
  record(value: unknown, finished: boolean): Promise<void>;
  /** Canvas x of the window's left edge as the page believes it (the native canvas must agree). */
  canvasV(): number;
  /** Requests whose arrival bubble is still in flight. */
  arrivals(): number;
  /** A silent run never takes the keyboard focus (the default for scripted checks); real app switches are skipped. */
  silent: boolean;
  /** Whether the page believes AskHuman is the active app (set by the focus events, or by a test). */
  appActive(): boolean;
  setAppActive(on: boolean): void;
  /** Feed one native drag-drop event to the page (what Tauri's `onDragDropEvent` would deliver); resolves when handled. */
  drag(payload: DragPayload): Promise<void>;
  /** Number of files dropped onto request `id` (all its questions), or onto its question `q`. */
  attachmentCount(id: number, q?: number): number;
  /** The question request `id` is on (the product's `current`). */
  currentQuestion(id: number): number;
  /** Hand the activation to another app, like a click elsewhere (resolves to whether macOS accepted it). */
  deactivate(): Promise<boolean>;
  /** Bring AskHuman back to the front (the user clicks the window again). */
  activate(): Promise<void>;
  /** Awaited between the offscreen layout and the native commit of a Sidebar growth. */
  setGrowHook(hook: ((step: "layout" | "commit") => Promise<void>) | null): void;
  /** Store a capture of the answer window under `label` (and write a PNG when `path` is given). */
  capture(label: string, path?: string): Promise<void>;
  /** Compare a screen-space region (x, top, width, height; AppKit coordinates) of two captures. */
  diff(a: string, b: string, region: [number, number, number, number]): Promise<RegionDiff>;
  /** Capture the window continuously into the shots directory (`<name>/frame-NNNN.png` + `index.json`). */
  burstStart(name: string, maxFrames: number, pauseMicroseconds?: number, writeAll?: boolean): Promise<void>;
  /** Experiments: choose how native frames are handed to the window server. */
  setSync(mode: string | undefined): void;
  /** Isolation switches: no page transitions, page content hidden, window shadow forced on / off (undefined = leave). */
  /** Reference animation on a plain window ("driver" | "appkit" | "layer"); sizes = from w, from h, to w, to h (points). */
  control(mode: string, cycles: number, restMs: number, sizes: [number, number, number, number]): Promise<void>;
  setDebug(flags: Partial<{ noMotion: boolean; noAnim: boolean; hidden: boolean; shadow: boolean | undefined }>): void;
  /** Plain-colour window behind the answer window for outside screen recordings (x, y, w, h in points, top-left origin). */
  backdrop(enable: boolean, region: [number, number, number, number]): Promise<void>;
  /** Stop the burst; resolves to the frames captured and how many of them were torn (see-through right / bottom edge). */
  burstStop(): Promise<{ frames: number; tears: number }>;
  /** Click the first element matching `selector` inside the stage like a user would; false if it is missing or disabled. */
  click(selector: string): boolean;
  /** Viewport rectangle of the first element matching `selector` inside the stage. */
  rect(selector: string): { left: number; top: number; width: number; height: number } | null;
}

interface Check { name: string; pass: boolean; detail: string }

const MAIN_MIN = { w: 420, h: 480 };
const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));
const near = (a: number, b: number, tolerance = 1) => Math.abs(a - b) <= tolerance;
const round = (n: number) => Math.round(n * 10) / 10;

async function until(condition: () => boolean, timeoutMs = 8000) {
  const start = performance.now();
  while (!condition()) {
    if (performance.now() - start > timeoutMs) return false;
    await sleep(16);
  }
  return true;
}

export async function runScenario(api: StageApi) {
  const results: Check[] = [];
  const check = (name: string, pass: boolean, detail = "") => { results.push({ name, pass, detail }); };
  const step = (name: string) => api.record({ label: "step", name }, false);
  const rect = (i: NativeInfo) => `${round(i.x)},${round(i.top)} ${round(i.width)}×${round(i.height)}`;
  const info = async () => (await api.info())!;
  const settled = async (label: string) => {
    check(`${label}: transition settles`, await until(() => api.snapshot().phase === "idle"));
    const now = await info();
    check(`${label}: page and native agree on the viewport origin`, near(now.canvasX, -api.canvasV(), 0.5),
      `native ${round(now.canvasX)} page ${-api.canvasV()}`);
  };
  /** Screen-space region (AppKit coordinates) of a page element, for comparing captures of a moving window. */
  const regionOf = (i: NativeInfo, selector: string): [number, number, number, number] | null => {
    const box = api.rect(selector);
    return box ? [i.x + i.canvasX + box.left, i.top - box.top, box.width, box.height] : null;
  };

  try {
    await step("start");
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });
    await sleep(100);

    // ---- S1: two requests, Sidebar visible
    await step("S1");
    await api.perform({ type: "reset", count: 2 });
    const full = await info();
    check("start: expanded with Sidebar", api.snapshot().view === "expanded" && near(full.width, 560 + api.sidebarExtra()),
      rect(full));
    check("start: lights visible, resizable, responsive viewport", !full.lightsHidden && full.resizable && full.canvasResponsive);
    check("start: always on top", full.alwaysOnTop);

    await api.perform({ type: "putAway" });
    let snap = api.snapshot();
    check("put away #1 switches to #2 and stays expanded", snap.view === "expanded" && snap.active === 2 && snap.requests[0].secondary);
    const switched = await info();
    check("switching bodies leaves the window untouched",
      near(switched.width, full.width) && near(switched.height, full.height) && near(switched.x, full.x) && near(switched.top, full.top),
      rect(switched));

    await api.perform({ type: "putAway" });
    snap = api.snapshot();
    check("put away #2 collapses the window", snap.view === "collapsed");
    await settled("collapse");
    const small = await info();
    const mini = api.mini();
    const collapse = api.lastReport();
    check("collapse: top-left anchored", near(small.x, full.x) && near(small.top, full.top), `${rect(full)} → ${rect(small)}`);
    check("collapse: reaches the measured small size", near(small.width, mini.w) && near(small.height, mini.h), `${rect(small)} vs ${mini.w}×${mini.h}`);
    check("collapse: lights hidden and not resizable", small.lightsHidden && !small.resizable);
    check("collapse: viewport pinned", small.canvasInstalled && !small.canvasResponsive);
    check("collapse: always-on-top preserved", small.alwaysOnTop);
    check("collapse: animated frames", (collapse?.report.frames ?? 0) >= 8, `${collapse?.report.frames ?? 0} frames`);
    check("collapse: geometry error", (collapse?.report.maxErrorPx ?? 99) <= 1.5, `${round(collapse?.report.maxErrorPx ?? 99)}px`);

    // ---- S2: a new request expands the small window
    await step("S2");
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("expand");
    snap = api.snapshot();
    const expanded = await info();
    check("arrival while small expands onto the new request", snap.view === "expanded" && snap.active === 3);
    check("expand: remembered size restored", near(expanded.width, full.width) && near(expanded.height, full.height), rect(expanded));
    check("expand: top-left anchored", near(expanded.x, full.x) && near(expanded.top, full.top), rect(expanded));
    check("expand: lights, resizing and viewport restored", !expanded.lightsHidden && expanded.resizable && expanded.canvasResponsive);
    check("expand: minimum size restored", expanded.minW >= MAIN_MIN.w - 0.5 && expanded.minH >= MAIN_MIN.h - 0.5, `${expanded.minW}×${expanded.minH}`);
    check("expand: animated frames", (api.lastReport()?.report.frames ?? 0) >= 8, `${api.lastReport()?.report.frames ?? 0} frames`);

    // ---- S3: interrupt a collapse halfway with an arrival
    await step("S3");
    const collapsing = api.perform({ type: "markAll" });
    await sleep(170);
    await api.perform({ type: "arrive", kind: "permission" });
    await collapsing;
    await settled("interrupted collapse");
    snap = api.snapshot();
    const reopened = await info();
    check("interrupted collapse ends expanded on the new request", snap.view === "expanded" && snap.active === 4);
    check("interrupted collapse: size restored", near(reopened.width, full.width) && near(reopened.height, full.height), rect(reopened));
    check("interrupted collapse: top-left anchored", near(reopened.x, full.x) && near(reopened.top, full.top), rect(reopened));
    check("interrupted collapse: window state restored", !reopened.lightsHidden && reopened.resizable && reopened.canvasResponsive);

    // ---- S4: interrupt an expand halfway with a put-away; the remembered size must survive
    await step("S4");
    await api.perform({ type: "markAll" });
    await settled("collapse again");
    check("collapsed again", api.snapshot().view === "collapsed");
    const expanding = api.perform({ type: "arrive", kind: "ask" });
    await sleep(170);
    await api.perform({ type: "putAway" });
    await expanding;
    await settled("interrupted expand");
    const resmall = await info();
    check("interrupted expand ends small", api.snapshot().view === "collapsed" && near(resmall.width, api.mini().w), rect(resmall));
    check("interrupted expand: top-left anchored", near(resmall.x, full.x) && near(resmall.top, full.top), rect(resmall));
    await api.perform({ type: "expandDefault" });
    await settled("expand after interruption");
    const back = await info();
    check("remembered size survives an interrupted expand", near(back.width, full.width) && near(back.height, full.height), rect(back));

    // ---- S5: one request (no Sidebar) collapses by cross-fade
    await step("S5");
    await api.perform({ type: "reset", count: 1 });
    const single = await info();
    check("single request: no Sidebar", near(single.width, 560), rect(single));
    await api.perform({ type: "putAway" });
    await settled("single collapse");
    const singleSmall = await info();
    check("single request collapses", api.snapshot().view === "collapsed" && near(singleSmall.width, api.mini().w) && near(singleSmall.height, api.mini().h), rect(singleSmall));
    check("single collapse: top-left anchored", near(singleSmall.x, single.x) && near(singleSmall.top, single.top));
    await api.perform({ type: "expandDefault" });
    await settled("single expand");
    const singleBack = await info();
    check("single request expands back", api.snapshot().view === "expanded" && near(singleBack.width, single.width) && near(singleBack.height, single.height), rect(singleBack));

    // ---- S6: the Sidebar appears next to an unchanged body (window grows to the left)
    await step("S6");
    const before = await info();
    const body = regionOf(before, ".pc-bodies");
    // The bubble would paint over the body region: this step only judges the layout protocol.
    api.tuning.arrivalBubble = false;
    await sleep(80);
    await api.capture("grow-before", "grow-1-before.png");
    api.setGrowHook(async growStep => {
      if (growStep !== "layout") return;
      await sleep(80);
      await api.capture("grow-layout", "grow-2-layout.png");
    });
    await api.perform({ type: "arrive", kind: "ask" });
    api.setGrowHook(null);
    await settled("Sidebar growth");
    await sleep(120);
    await api.capture("grow-after", "grow-3-after.png");
    api.tuning.arrivalBubble = true;
    const grown = await info();
    const extra = api.sidebarExtra();
    if (body) {
      // The product's guarantee (PopupInboxView.geometry): switching the page to the final layout changes
      // nothing visible (the new column is still offscreen), and the one native commit keeps the body put.
      const layoutStep = await api.diff("grow-before", "grow-layout", body);
      const commitStep = await api.diff("grow-before", "grow-after", body);
      check("Sidebar growth: the offscreen layout step changes nothing visible",
        layoutStep.changedFraction < 0.002, `${(layoutStep.changedFraction * 100).toFixed(3)}% changed, mean ${layoutStep.meanAbs.toFixed(2)}`);
      check("Sidebar growth: the body is pixel-stable across the commit",
        commitStep.changedFraction < 0.002, `${(commitStep.changedFraction * 100).toFixed(3)}% changed, mean ${commitStep.meanAbs.toFixed(2)}`);
    } else {
      check("Sidebar growth: body region found", false);
    }
    const column = regionOf(grown, ".pane-sidebar");
    if (column) {
      const painted = await api.diff("grow-after", "grow-after", [column[0], column[1] - 40, column[2], 120]);
      check("Sidebar growth: the revealed column is painted at once", painted.stddevA > 6, `luma stddev ${painted.stddevA.toFixed(1)}`);
    }
    check("Sidebar growth: widens by the Sidebar", near(grown.width, before.width + extra), `${rect(before)} → ${rect(grown)}`);
    check("Sidebar growth: the body stays put (window grows left)", near(grown.x, before.x - extra) && near(grown.top, before.top), rect(grown));
    check("Sidebar growth: height unchanged and viewport responsive", near(grown.height, before.height) && grown.canvasResponsive);

    // ---- S7: always-on-top follows the user's setting across transitions
    await step("S7");
    await api.perform({ type: "pin", on: false });
    check("pin off applied", !(await info()).alwaysOnTop);
    await api.perform({ type: "markAll" });
    await settled("collapse unpinned");
    check("collapsed window stays unpinned", !(await info()).alwaysOnTop);
    await api.perform({ type: "expandDefault" });
    await settled("expand unpinned");
    check("expanded window stays unpinned", !(await info()).alwaysOnTop);
    await api.perform({ type: "pin", on: true });
    check("pin on applied", (await info()).alwaysOnTop);


    // ---- S9: a user working only with the mouse while the Sidebar is present
    await step("S9");
    const flag = (id: number) => `.inbox-entry:has([data-inbox-row="${id}"]) .row-flag`;
    const row = (id: number) => `[data-inbox-row="${id}"]`;
    await api.perform({ type: "reset", count: 3 });
    check("mouse: the Sidebar row action for #2 exists", api.click(flag(2)));
    await sleep(200);
    snap = api.snapshot();
    check("mouse: marking another row changes only its mark", snap.requests[1].secondary && snap.view === "expanded" && snap.active === 1);
    api.click(flag(3));
    await sleep(200);
    snap = api.snapshot();
    check("mouse: marking the last other row keeps the active request open", snap.requests[2].secondary && snap.view === "expanded" && !snap.requests[0].secondary);
    check("mouse: the footer button puts the active request away", api.click('[data-act="put-away"]'));
    await settled("mouse: footer put away");
    snap = api.snapshot();
    const mouseSmall = await info();
    check("mouse: everything secondary collapses the window", snap.view === "collapsed" && near(mouseSmall.width, api.mini().w) && near(mouseSmall.height, api.mini().h), rect(mouseSmall));
    check("mouse: a small-list row can be opened", api.click(row(2)));
    await settled("mouse: open a secondary row");
    snap = api.snapshot();
    check("mouse: opening a secondary row expands and keeps its mark", snap.view === "expanded" && snap.active === 2 && snap.requests[1].secondary);
    check("mouse: the open row can be restored from the Sidebar", api.click(flag(2)));
    await sleep(250);
    snap = api.snapshot();
    check("mouse: restoring the open row keeps the window open", snap.view === "expanded" && snap.active === 2 && !snap.requests[1].secondary);
    check("mouse: the footer button on it collapses again", api.click('[data-act="put-away"]'));
    await settled("mouse: collapse again");
    check("mouse: back to the small window", api.snapshot().view === "collapsed");
    check("mouse: a small-list row can be restored directly", api.click(flag(3)));
    await settled("mouse: restore from the small window");
    snap = api.snapshot();
    check("mouse: restoring from the small window expands onto that request", snap.view === "expanded" && snap.active === 3 && !snap.requests[2].secondary);
    check("mouse: marking the open row from the Sidebar collapses (it is the only primary)", api.click(flag(3)));
    await settled("mouse: mark the open row");
    check("mouse: the window is small again", api.snapshot().view === "collapsed");

    // The report "once the Sidebar has appeared the collapsed state can no longer be reached": a single request
    // is put away, a newcomer expands the window (the Sidebar appears) and the user puts that one away too.
    await api.perform({ type: "reset", count: 1 });
    api.click('[data-act="put-away"]');
    await settled("sidebar-later: single put away");
    check("sidebar-later: the lone request collapses", api.snapshot().view === "collapsed");
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("sidebar-later: arrival");
    snap = api.snapshot();
    const withSidebar = await info();
    check("sidebar-later: the newcomer expands the window with the Sidebar", snap.view === "expanded" && snap.active === 2 && near(withSidebar.width, 560 + api.sidebarExtra()), rect(withSidebar));
    api.click('[data-act="put-away"]');
    await settled("sidebar-later: put away the newcomer");
    check("sidebar-later: the collapsed state is reachable again", api.snapshot().view === "collapsed");
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("sidebar-later: second arrival");
    api.click('[data-act="put-away"]');
    await settled("sidebar-later: put away again");
    check("sidebar-later: and again with three requests", api.snapshot().view === "collapsed");

    // ---- S10: the arrival bubble follows an arrival that expands the small window
    await step("S10");
    await api.perform({ type: "reset", count: 1 });
    api.click('[data-act="put-away"]');
    await settled("bubble: put away");
    await api.perform({ type: "arrive", kind: "ask" });
    check("bubble: an arrival that expands the small window plays the bubble", api.arrivals() === 1 && api.snapshot().view === "expanded");
    await sleep(750);
    await api.capture("bubble-hop", "bubble-1-hop.png");
    await sleep(1500);
    await api.capture("bubble-fly", "bubble-2-fly.png");
    check("bubble: lands in the row within the product timing", await until(() => api.arrivals() === 0, 5000));
    await sleep(150);
    await api.capture("bubble-landed", "bubble-3-landed.png");

    // The common path: a second request arrives while the single window is open (the Sidebar grows, then the bubble flies).
    await api.perform({ type: "reset", count: 1 });
    await api.perform({ type: "arrive", kind: "ask" });
    snap = api.snapshot();
    check("bubble: a second request at an open window grows the Sidebar, keeps the body and plays the bubble",
      snap.view === "expanded" && snap.active === 1 && api.arrivals() === 1);
    await sleep(750);
    await api.capture("bubble-growth-hop", "bubble-4-growth-hop.png");
    check("bubble: that one lands too", await until(() => api.arrivals() === 0, 5000));

    api.tuning.arrivalBody = "keep";
    await api.perform({ type: "reset", count: 1 });
    api.click('[data-act="put-away"]');
    await settled("keep: put away");
    await api.perform({ type: "arrive", kind: "ask" });
    snap = api.snapshot();
    check("keep: the window expands onto the old body and the newcomer stays unread",
      snap.view === "expanded" && snap.active === 1 && snap.requests[1].visited === false);
    check("keep: the bubble still plays for the unread newcomer", api.arrivals() === 1);
    check("keep: the bubble lands", await until(() => api.arrivals() === 0, 5000));
    api.tuning.arrivalBody = "new";

    // ---- S11: the collapsed state stays reachable under every look the controller can select
    await step("S11");
    const looks: { name: string; tuning: Partial<Tuning> }[] = [
      { name: "capsule", tuning: { form: "capsule" } },
      { name: "rows sink", tuning: { secondaryOrder: "bottom" } },
      { name: "lights kept", tuning: { lights: "keep" } },
      { name: "full rows", tuning: { density: "same", rows: 2 } },
      { name: "capsule+sink+lights", tuning: { form: "capsule", secondaryOrder: "bottom", lights: "keep" } },
      { name: "high+chevrons", tuning: { salience: "high", rowAction: "chevron" } },
      { name: "low+clock", tuning: { salience: "low", rowAction: "clock" } },
    ];
    for (const look of looks) {
      Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 }, look.tuning);
      await sleep(250);
      await api.perform({ type: "reset", count: 3 });
      api.click(flag(2));
      api.click(flag(3));
      await sleep(200);
      api.click('[data-act="put-away"]');
      await settled(`look ${look.name}: collapse`);
      check(`look ${look.name}: everything secondary collapses`, api.snapshot().view === "collapsed");
      await api.perform({ type: "expandDefault" });
      await settled(`look ${look.name}: expand`);
      check(`look ${look.name}: expands again`, api.snapshot().view === "expanded");
      // The window shows a 稍后答 request: leaving AskHuman (C23) is what collapses it again.
      await api.perform({ type: "leave" });
      await settled(`look ${look.name}: collapse once more`);
      check(`look ${look.name}: collapses once more`, api.snapshot().view === "collapsed");
    }
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });

    // ---- S12: stills of the Sidebar row action variants (hover state forced on) for visual review
    await step("S12");
    for (const action of ["archive", "text", "clock", "chevron"] as const) {
      Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1, rowAction: action });
      await sleep(200);
      await api.perform({ type: "reset", count: 3 });
      api.click(flag(2));
      await sleep(250);
      // At rest: the archive box shows the standing "稍后答" label on the row that was put away, the others nothing.
      if (action === "archive") await api.capture("rowaction-archive-idle", "rowaction-archive-idle.png");
      document.querySelector(".stage")?.setAttribute("data-force-flags", "1");
      await sleep(250);
      await api.capture(`rowaction-${action}`, `rowaction-${action}.png`);
      document.querySelector(".stage")?.removeAttribute("data-force-flags");
    }
    check("row action stills captured", true);
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });

    // ---- S8: remote end of everything closes the round
    await step("S8");
    for (let i = 0; i < 6 && !api.snapshot().closed; i++) await api.perform({ type: "endActive" });
    check("round closes when the queue empties", api.snapshot().closed);

    // ---- S13: a request that arrives after the round ended is an ordinary first popup again (the report
    // "after everything was answered a new request showed no window; the second one did, at stale geometry")
    await step("S13");
    await api.perform({ type: "arrive", kind: "ask" });
    snap = api.snapshot();
    let reborn = await info();
    check("after the round: the new request opens a fresh window", !snap.closed && snap.view === "expanded" && snap.requests.length === 1 && snap.active === 1 && reborn.visible,
      `visible ${reborn.visible}, ${snap.requests.length} request(s)`);
    check("after the round: ordinary single popup (normal size, no Sidebar)", near(reborn.width, 560) && near(reborn.height, 652), rect(reborn));
    check("after the round: lights, resizing, viewport and pin in the normal state",
      !reborn.lightsHidden && reborn.resizable && reborn.canvasResponsive && reborn.alwaysOnTop);
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("after the round: second arrival");
    reborn = await info();
    check("after the round: the second request adds the Sidebar", api.snapshot().requests.length === 2 && near(reborn.width, 560 + api.sidebarExtra()), rect(reborn));
    await api.perform({ type: "markAll" });
    await settled("after the round: collapse");
    check("after the round: the new round collapses like any other", api.snapshot().view === "collapsed");
    // The round ends while the window is small; the next request must not inherit the small window.
    for (let i = 0; i < 4 && !api.snapshot().closed; i++) await api.perform({ type: "endLast" });
    check("ended while small: the round closes", api.snapshot().closed);
    await api.perform({ type: "arrive", kind: "permission" });
    snap = api.snapshot();
    reborn = await info();
    check("ended while small: the next request opens a normal window", !snap.closed && snap.view === "expanded" && snap.requests.length === 1 && reborn.visible
      && near(reborn.width, 560) && near(reborn.height, 652) && !reborn.lightsHidden && reborn.resizable && reborn.canvasResponsive, rect(reborn));
    // An arrival right behind the last answer (inside the short grace period before the window hides) keeps the window.
    const ending = api.perform({ type: "endLast" });
    await sleep(10);
    check("grace: the round is closing", api.snapshot().closed);
    await api.perform({ type: "arrive", kind: "ask" });
    await ending;
    await sleep(300);
    check("grace: an arrival during the grace period keeps the window shown", (await info())?.visible === true && !api.snapshot().closed && api.snapshot().requests.length === 1);

    // ---- S14: "稍后答" semantics. Answering leads on to the next 稍后答 request instead of collapsing (C22); the footer
    // button toggles between 稍后回答 and 取消稍后答 (C24); leaving AskHuman shrinks a peek, never a window with a
    // primary request (C23, simulated here — S15 does it for real).
    await step("S14");
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });
    await api.perform({ type: "reset", count: 3 });
    api.click(flag(2));
    api.click(flag(3));
    await sleep(200);
    api.click('[data-act="put-away"]');
    await settled("later: collapse");
    snap = api.snapshot();
    check("later: all three requests are 稍后答 in the small window", snap.view === "collapsed" && snap.requests.length === 3 && snap.requests.every(r => r.secondary));
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("later: the newcomer expands the window");
    snap = api.snapshot();
    const newcomerWindow = await info();
    check("later: the newcomer expands the window and is shown", snap.view === "expanded" && snap.active === 4 && !snap.requests[3].secondary);
    check("later: an option can be chosen", api.click(".pc-option"));
    await sleep(100);
    await api.perform({ type: "answer" });
    await settled("later: answered");
    snap = api.snapshot();
    const afterAnswer = await info();
    const laterOpen = snap.requests.find(r => r.id === snap.active);
    check("later: answering the newcomer moves on to a 稍后答 request and keeps the window expanded",
      snap.view === "expanded" && snap.requests.length === 3 && laterOpen?.secondary === true, `active ${snap.active}`);
    check("later: the window itself neither moves nor resizes",
      near(afterAnswer.width, newcomerWindow.width) && near(afterAnswer.height, newcomerWindow.height) && near(afterAnswer.x, newcomerWindow.x) && near(afterAnswer.top, newcomerWindow.top),
      `${rect(newcomerWindow)} → ${rect(afterAnswer)}`);

    const footerButton = () => document.querySelector<HTMLElement>('.stage .pc-footer [data-act="unmark"], .stage .pc-footer [data-act="put-away"]');
    check("later: the footer button of a 稍后答 request reads 取消稍后答",
      footerButton()?.dataset.act === "unmark" && footerButton()?.textContent?.trim() === "取消稍后答", footerButton()?.textContent?.trim());
    const openId = snap.active!;
    check("later: 取消稍后答 can be clicked", api.click('[data-act="unmark"]'));
    await sleep(250);
    snap = api.snapshot();
    check("later: 取消稍后答 restores the open request and keeps the window and the body",
      snap.view === "expanded" && snap.active === openId && snap.requests.find(r => r.id === openId)?.secondary === false);
    check("later: the footer button reads 稍后回答 again",
      footerButton()?.dataset.act === "put-away" && footerButton()?.textContent?.trim() === "稍后回答", footerButton()?.textContent?.trim());
    await api.perform({ type: "leave" });
    await sleep(400);
    check("later: leaving AskHuman never shrinks a window that has a primary request", api.snapshot().view === "expanded");
    api.click('[data-act="put-away"]');
    await settled("later: put away again");
    check("later: 稍后回答 on the last primary request collapses the window", api.snapshot().view === "collapsed");

    await api.perform({ type: "expandDefault" });
    await settled("later: peek");
    snap = api.snapshot();
    check("later: the small window opens as a peek (expanded, only 稍后答 requests)", snap.view === "expanded" && snap.requests.every(r => r.secondary));
    api.tuning.autoCollapseOnLeave = false;
    await api.perform({ type: "leave" });
    await sleep(400);
    check("later: with the setting off, leaving does nothing", api.snapshot().view === "expanded");
    api.tuning.autoCollapseOnLeave = true;
    await api.perform({ type: "leave" });
    await settled("later: leave");
    const away = await info();
    check("later: leaving AskHuman shrinks the peek back to the small window",
      api.snapshot().view === "collapsed" && near(away.width, api.mini().w) && near(away.height, api.mini().h), rect(away));
    check("later: every request keeps its 稍后答 mark", api.snapshot().requests.length === 3 && api.snapshot().requests.every(r => r.secondary));

    // A channel ended the open request: with AskHuman active the user is taken on to the next 稍后答 request,
    // with the user away nobody is looking and the window just collapses.
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("later: remote, user away");
    api.setAppActive(false);
    await api.perform({ type: "endActive" });
    await settled("later: remote end while away");
    check("later: a remote end while the user is away collapses instead of moving on", api.snapshot().view === "collapsed");
    api.setAppActive(true);
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("later: remote, user engaged");
    await api.perform({ type: "endActive" });
    await settled("later: remote end while engaged");
    snap = api.snapshot();
    check("later: a remote end while AskHuman is active moves on to a 稍后答 request",
      snap.view === "expanded" && snap.requests.find(r => r.id === snap.active)?.secondary === true, `active ${snap.active}`);

    // ---- S16: the way back to the small window by hand (C27). A peek has a "缩回小窗" icon in the Sidebar header (next to
    // the pin when the window has no Sidebar), in the slot the small window's "展开" icon uses. Nothing else holds a window
    // without a primary request open, so that is the only time it exists.
    await step("S16");
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });
    const iconShown = (selector: string) => {
      const element = document.querySelector<HTMLElement>(`.stage ${selector}`);
      if (!element) return false;
      const style = getComputedStyle(element);
      return style.display !== "none" && style.visibility !== "hidden";
    };
    await api.perform({ type: "reset", count: 3 });
    check("shrink icon: hidden while every request is primary", !iconShown(".mini-shrink") && !iconShown(".pc-shrink"));
    api.click(flag(2));
    api.click(flag(3));
    await sleep(200);
    check("shrink icon: hidden while the open request is still primary", !iconShown(".mini-shrink"));
    api.click('[data-act="put-away"]');
    await settled("shrink icon: collapse");
    await api.perform({ type: "expandDefault" });
    await settled("shrink icon: peek");
    check("shrink icon: a peek with a Sidebar shows it in the Sidebar header only",
      iconShown(".mini-shrink") && !iconShown(".pc-shrink") && !iconShown(".mini-expand"));
    await sleep(300);
    await api.capture("shrink-peek", "shrink-peek.png");
    const shrinkBox = api.rect(".mini-shrink");
    check("shrink icon: it can be clicked", api.click(".mini-shrink"));
    await settled("shrink icon: shrunk");
    snap = api.snapshot();
    const shrunk = await info();
    check("shrink icon: the click shrinks the peek to the small window",
      snap.view === "collapsed" && near(shrunk.width, api.mini().w) && near(shrunk.height, api.mini().h), rect(shrunk));
    check("shrink icon: every request keeps its 稍后答 mark", snap.requests.length === 3 && snap.requests.every(r => r.secondary));
    const expandBox = api.rect(".mini-expand");
    check("shrink icon: the small window's expand icon takes the same slot (same x)",
      !!shrinkBox && !!expandBox && near(shrinkBox.left, expandBox.left, 1.5), `${shrinkBox?.left} vs ${expandBox?.left}`);
    check("shrink icon: hidden again in the small window", !iconShown(".mini-shrink") && iconShown(".mini-expand"));
    await api.perform({ type: "expandDefault" });
    await settled("shrink icon: peek again");
    await api.perform({ type: "arrive", kind: "ask" });
    await settled("shrink icon: newcomer");
    check("shrink icon: a new primary request takes it away", api.snapshot().view === "expanded" && !iconShown(".mini-shrink"));

    await api.perform({ type: "reset", count: 1 });
    api.click('[data-act="put-away"]');
    await settled("shrink icon: single collapse");
    await api.perform({ type: "expandDefault" });
    await settled("shrink icon: single peek");
    check("shrink icon: a peek without a Sidebar shows it next to the pin instead", iconShown(".pc-shrink") && !iconShown(".mini-shrink"));
    await sleep(300);
    await api.capture("shrink-peek-single", "shrink-peek-single.png");
    check("shrink icon: that one can be clicked", api.click(".pc-shrink"));
    await settled("shrink icon: single shrunk");
    check("shrink icon: the single-request peek shrinks too", api.snapshot().view === "collapsed");

    // ---- S17: files dragged over the window (C28, C31), like dragging onto spring-loaded folders in Finder. Resting over the
    // small window opens it onto the row under the pointer (anywhere else on it: the default request), resting on another
    // Sidebar row switches the body, and a drop attaches the files to whatever is under the pointer without waiting for any
    // of that. The events are fed to the page the way Tauri delivers them (wry/macOS: x from the window's left edge, y down
    // from the top of the webview *frame*, which sticks out below a window whose viewport is pinned larger, like the small
    // window). A real drag from Finder is for the user to try: this covers the page logic and the coordinate correction.
    await step("S17");
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1, dragSwitchMs: 200, dragExpandMs: 300 });
    /** Page coordinates → what wry would report for them. */
    const pointerAt = async (x: number, y: number) => {
      const now = await info();
      return { x: x - api.canvasV(), y: y + Math.max(0, now.viewH - now.height) };
    };
    const pointer = async (selector: string) => {
      const box = api.rect(selector);
      return box ? pointerAt(box.left + box.width / 2, box.top + box.height / 2) : { x: 0, y: 0 };
    };
    const lit = () => document.querySelector(".stage .inbox-row.drop-target")?.getAttribute("data-inbox-row") ?? null;
    const waiting = () => document.querySelector(".stage .inbox-row.dwelling")?.getAttribute("data-inbox-row") ?? null;
    const dragging = () => document.querySelector(".stage")?.getAttribute("data-dragging");
    const labels = (id: number) => [...document.querySelectorAll(`.stage [data-inbox-row="${id}"] em`)].map(e => e.textContent);
    const files = ["/Users/me/Pictures/a.png", "/Users/me/Desktop/b.jpg"];
    /** The pointer moves over `selector`; the page needs a frame to put the highlight into the DOM. */
    const over = async (selector: string) => {
      await api.drag({ type: "over", position: await pointer(selector) });
      await sleep(20);
    };
    await api.perform({ type: "reset", count: 3 });
    const regular = await info();
    api.click(flag(2));
    api.click(flag(3));
    await sleep(200);
    api.click('[data-act="put-away"]');
    await settled("drag: collapse");
    snap = api.snapshot();
    const tiny = await info();
    check("drag: three 稍后答 requests in the small window", snap.view === "collapsed" && snap.requests.every(r => r.secondary));
    check("drag: the small window's viewport sticks out below it (the case the y correction is for)",
      tiny.viewH > tiny.height + 50, `viewport ${round(tiny.viewH)} window ${round(tiny.height)}`);

    // A short pass over the small window opens nothing. The row under the pointer shows what the wait will open.
    await over(row(2));
    check("drag: the row under the pointer in the small window is lit and shows the wait", lit() === "2" && waiting() === "2", `lit ${lit()} wait ${waiting()}`);
    await sleep(150);
    await api.capture("drag-small", "drag-small.png");
    await api.drag({ type: "leave" });
    await sleep(450);
    check("drag: a short pass over the small window opens nothing", api.snapshot().view === "collapsed" && waiting() === null);
    // Anywhere else on the small window the wait is for the default request.
    await over(".inbox-heading");
    check("drag: resting on the heading waits for the default request", waiting() === "1" && lit() === null, `wait ${waiting()} lit ${lit()}`);
    await api.drag({ type: "leave" });
    await sleep(450);
    check("drag: and opens nothing when the pointer leaves", api.snapshot().view === "collapsed");

    // Real events arrive as a burst: `enter`, `over` a millisecond later, then more every few ms while the pointer rests.
    // Resting on the row of request 3 opens the window onto it; nothing may be lost on the way (a lost `enter` once left the
    // window shut for good).
    const entry = await pointer(row(3));
    const moving = setInterval(() => void api.drag({ type: "over", position: entry }), 16);
    await Promise.all([
      api.drag({ type: "enter", paths: files, position: entry }),
      api.drag({ type: "over", position: entry }),
      api.drag({ type: "over", position: entry }),
    ]);
    check("drag: the burst alone has not opened the window yet", api.snapshot().view === "collapsed");
    check("drag: the window opens once the pointer has rested long enough", await until(() => api.snapshot().view === "expanded", 3000));
    await settled("drag: opened by resting");
    // The page keeps hearing the same, unmoved position for a while.
    await sleep(500);
    clearInterval(moving);
    snap = api.snapshot();
    const opened = await info();
    check("drag: resting on a row of the small window opens that request", snap.view === "expanded" && snap.active === 3, `active ${snap.active}`);
    check("drag: the window comes back at its remembered size, top-left anchored",
      near(opened.width, regular.width) && near(opened.height, regular.height) && near(opened.x, regular.x) && near(opened.top, regular.top), rect(opened));
    check("drag: a pointer that has not moved starts no new wait once the window opened", waiting() === null && snap.active === 3);
    check("drag: the window is in the dragging state", dragging() === "yes");

    // The wait is parked far away while the still is taken, so that the capture does not switch the body.
    api.tuning.dragSwitchMs = 5000;
    await over(row(1));
    check("drag: the row under the pointer is lit as the drop target", lit() === "1", `${lit()}`);
    await sleep(150);
    await api.capture("drag-row-lit", "drag-row-lit.png");
    check("drag: the body has not switched yet", api.snapshot().active === 3);
    api.tuning.dragSwitchMs = 200;
    await sleep(40);
    await over(row(2));
    check("drag: moving on lights the next row", lit() === "2", `${lit()}`);
    check("drag: and shows the wait on it", waiting() === "2", `${waiting()}`);
    await sleep(60);
    check("drag: a rest shorter than the wait switches nothing", api.snapshot().active === 3);
    await sleep(320);
    check("drag: resting on a row switches the body to its request", api.snapshot().active === 2, `active ${api.snapshot().active}`);
    check("drag: switching bodies leaves the window alone",
      near((await info()).width, opened.width) && near((await info()).height, opened.height));
    await over(".pc-bodies");
    check("drag: over the answer area the whole area is the target",
      !!document.querySelector(".stage .pane-main.drop-target") && lit() === null);
    check("drag: and its label says where the files go",
      document.querySelector(".stage .pane-main")?.getAttribute("data-drop-label") === "松手添加到当前请求");
    await sleep(200);
    await api.capture("drag-main-lit", "drag-main-lit.png");

    await over(row(3));
    await api.drag({ type: "drop", paths: files, position: await pointer(row(3)) });
    snap = api.snapshot();
    check("drag: a drop on a row attaches the files to that request at once, without waiting for the switch",
      api.attachmentCount(3) === 2 && api.attachmentCount(2) === 0 && api.attachmentCount(1) === 0,
      `#1 ${api.attachmentCount(1)}, #2 ${api.attachmentCount(2)}, #3 ${api.attachmentCount(3)}`);
    check("drag: and shows that request, still marked 稍后答", snap.view === "expanded" && snap.active === 3 && snap.requests[2].secondary);
    check("drag: the drag state is over", dragging() === "no" && lit() === null && waiting() === null && !document.querySelector(".stage .pane-main.drop-target"));
    await sleep(300);
    check("drag: the body lists the files", document.querySelectorAll(".stage .pc-attachment").length === 2);
    check("drag: the row says how many files it holds", labels(3).includes("附件 2"), `${labels(3)}`);
    await api.capture("drag-dropped", "drag-dropped.png");
    await api.drag({ type: "enter", paths: [files[0]], position: await pointer(".pc-bodies") });
    await api.drag({ type: "drop", paths: [files[0]], position: await pointer(".pc-bodies") });
    check("drag: a drop on the answer area attaches to the open request", api.attachmentCount(3) === 3);
    check("drag: an attachment can be removed again", api.click(".pc-attachment button") && (await until(() => api.attachmentCount(3) === 2, 1000)));

    // From here on the small window opens at once (wait 0), which is also how the page used to behave: the checks below are
    // about spring-back and about what has to survive the burst, not about the wait.
    api.tuning.dragExpandMs = 0;
    await api.perform({ type: "leave" });
    await settled("drag: leave");
    check("drag: the peek shrinks when the user leaves", api.snapshot().view === "collapsed");
    // The burst, with no wait: nothing may be lost on the way.
    const rushed = await pointer(row(1));
    await Promise.all([
      api.drag({ type: "enter", paths: files, position: rushed }),
      api.drag({ type: "over", position: rushed }),
      api.drag({ type: "over", position: rushed }),
    ]);
    await settled("drag: opened at once");
    check("drag: with no wait the burst opens the window onto the row under the pointer", api.snapshot().view === "expanded" && api.snapshot().active === 1);
    await api.drag({ type: "leave" });
    check("drag: a drag that gives up shrinks the window back", await until(() => api.snapshot().view === "collapsed", 4000));
    await settled("drag: sprang back");
    await api.drag({ type: "enter", paths: files, position: await pointer(row(1)) });
    await settled("drag: opened a third time");
    await api.drag({ type: "leave" });
    await sleep(250);
    await api.drag({ type: "enter", paths: files, position: await pointer(row(2)) });
    await sleep(900);
    check("drag: coming back within the grace period keeps the window", api.snapshot().view === "expanded");
    await api.drag({ type: "leave" });
    check("drag: and giving up again shrinks it", await until(() => api.snapshot().view === "collapsed", 4000));
    await settled("drag: sprang back again");
    api.tuning.dragSpringBack = false;
    await api.drag({ type: "enter", paths: files, position: await pointer(row(1)) });
    await settled("drag: opened with spring-back off");
    await api.drag({ type: "leave" });
    await sleep(900);
    check("drag: with spring-back off the window stays", api.snapshot().view === "expanded");
    api.tuning.dragSpringBack = true;
    await api.perform({ type: "shrink" });
    await settled("drag: shrink by hand");

    // A newcomer that arrives meanwhile is primary: the window must not close under it.
    await api.drag({ type: "enter", paths: files, position: await pointer(row(1)) });
    await settled("drag: opened for the newcomer test");
    await api.perform({ type: "arrive", kind: "ask" });
    await api.drag({ type: "leave" });
    await sleep(900);
    check("drag: a primary request that arrived during the drag keeps the window open", api.snapshot().view === "expanded");

    // Dropped onto the small window itself (no `enter` first): opens the request under the pointer, files attached.
    await api.perform({ type: "reset", count: 2 });
    api.click(flag(2));
    await sleep(150);
    api.click('[data-act="put-away"]');
    await settled("drag: small again");
    // The `enter` can be missed; a pointer move over the small window opens it just the same.
    await api.drag({ type: "over", position: await pointer(row(2)) });
    await settled("drag: opened by a move alone");
    check("drag: a pointer move alone expands the small window", api.snapshot().view === "expanded");
    await api.drag({ type: "leave" });
    check("drag: and leaving again shrinks it", await until(() => api.snapshot().view === "collapsed", 4000));
    await settled("drag: small once more");
    await api.drag({ type: "drop", paths: files, position: await pointer(row(2)) });
    await settled("drag: dropped on the small window");
    snap = api.snapshot();
    check("drag: a drop on a row of the small window attaches and opens that request",
      api.attachmentCount(2) === 2 && snap.view === "expanded" && snap.active === 2, `active ${snap.active}, view ${snap.view}`);
    // On the heading (no row under the pointer) the files go to the request the small window would open.
    await api.perform({ type: "shrink" });
    await settled("drag: small for the heading drop");
    await api.drag({ type: "drop", paths: [files[0]], position: await pointer(".inbox-heading") });
    await settled("drag: dropped on the heading");
    snap = api.snapshot();
    check("drag: a drop on the heading attaches to the default request and opens it",
      snap.view === "expanded" && snap.active === 2 && api.attachmentCount(2) === 3, `active ${snap.active}, files ${api.attachmentCount(2)}`);

    // ---- S18: requests with several questions (C32). Sequential layout (the product's default): one question at a time, files go
    // to the current one. Vertical layout: every question is a card, the card under the pointer is the target, and the pointer
    // at the top / bottom edge of the body scrolls it so that every card can be reached. The Sidebar list scrolls the same way.
    await step("S18");
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1, dragSwitchMs: 200, dragExpandMs: 0, arrivalBubble: false });
    await api.perform({ type: "reset", count: 1 });
    await api.perform({ type: "arrive", kind: "ask", multi: true });
    await settled("multi: arrived");
    api.click(row(2));
    await sleep(350);
    const cards = () => document.querySelectorAll(".stage .pc-question").length;
    const questionHead = () => document.querySelector(".stage .pc-qhead")?.textContent?.replace(/\s+/g, " ").trim();
    const stepButton = (act: string) => document.querySelector<HTMLButtonElement>(`.stage [data-act="${act}"]`);
    const mainLabel = () => document.querySelector(".stage .pane-main")?.getAttribute("data-drop-label");
    check("multi: the request with three questions is open", api.snapshot().active === 2 && !!document.querySelector(".stage .pc-qtext"));
    check("multi: sequential layout shows one question at a time", cards() === 1 && questionHead() === "第 1 / 3 题", `${cards()} ${questionHead()}`);
    check("multi: the footer steps through the questions", !!stepButton("next-q") && !!stepButton("prev-q") && stepButton("prev-q")!.disabled && !stepButton("next-q")!.disabled);

    await api.drag({ type: "enter", paths: files, position: await pointer(".pc-bodies") });
    await sleep(40);
    check("multi: the label over the body names the question the files would go to", mainLabel() === "松手添加到第 1 题", `${mainLabel()}`);
    await api.drag({ type: "drop", paths: files, position: await pointer(".pc-bodies") });
    check("multi: sequential: files dropped on the body go to the current (first) question",
      api.attachmentCount(2, 0) === 2 && api.attachmentCount(2, 1) === 0 && api.attachmentCount(2, 2) === 0,
      `${api.attachmentCount(2, 0)}/${api.attachmentCount(2, 1)}/${api.attachmentCount(2, 2)}`);
    check("multi: the row counts the files of all its questions", labels(2).includes("附件 2"), `${labels(2)}`);
    check("multi: the file sits in the first question", document.querySelectorAll(".stage .pc-attachment").length === 2);

    check("multi: 下一个 moves on to the second question", api.click('[data-act="next-q"]') && (await until(() => api.currentQuestion(2) === 1, 1000)));
    await sleep(100);
    check("multi: and the body shows it", questionHead() === "第 2 / 3 题" && cards() === 1, `${questionHead()}`);
    await api.drag({ type: "enter", paths: files, position: await pointer(".pc-bodies") });
    await sleep(40);
    check("multi: the label follows the current question", mainLabel() === "松手添加到第 2 题", `${mainLabel()}`);
    await sleep(150);
    await api.capture("multi-seq-lit", "multi-seq-lit.png");
    await api.drag({ type: "drop", paths: [files[0]], position: await pointer(".pc-bodies") });
    check("multi: files dropped on the body now go to the second question", api.attachmentCount(2, 1) === 1 && api.attachmentCount(2, 0) === 2);

    // Another request is open; a drop on this one's row goes to the question it is on.
    api.click(row(1));
    await sleep(350);
    check("multi: the single-question request shows no question header", api.snapshot().active === 1 && questionHead() === undefined);
    await api.drag({ type: "enter", paths: files, position: await pointer(row(2)) });
    await api.drag({ type: "drop", paths: files, position: await pointer(row(2)) });
    await sleep(350);
    check("multi: a drop on the row of a multi-question request goes to the question it is on",
      api.attachmentCount(2, 1) === 3 && api.attachmentCount(2, 0) === 2 && api.attachmentCount(2, 2) === 0 && api.snapshot().active === 2,
      `${api.attachmentCount(2, 0)}/${api.attachmentCount(2, 1)}/${api.attachmentCount(2, 2)}`);
    check("multi: and the body shows that question", questionHead() === "第 2 / 3 题", `${questionHead()}`);

    // Vertical layout.
    api.tuning.questionLayout = "vertical";
    await sleep(300);
    const bodyEl = () => document.querySelector<HTMLElement>(".stage .pc-bodies .pc-body")!;
    const cardSel = (i: number) => `.pc-question[data-q-index="${i}"]`;
    const litCard = () => document.querySelector(".stage .pc-question.card.drop-target")?.getAttribute("data-q-index") ?? null;
    check("multi: the vertical layout shows every question as a card", document.querySelectorAll(".stage .pc-question.card").length === 3);
    check("multi: it has no stepping footer", !stepButton("next-q") && !stepButton("prev-q"));
    check("multi: the cards overflow the body, which therefore scrolls", bodyEl().scrollHeight > bodyEl().clientHeight + 100, `${bodyEl().scrollHeight} vs ${bodyEl().clientHeight}`);
    bodyEl().scrollTop = 0;
    await sleep(60);
    await api.drag({ type: "enter", paths: files, position: await pointer(cardSel(0)) });
    await sleep(60);
    check("multi: vertical: the card under the pointer is the target", litCard() === "0" && !document.querySelector(".stage .pane-main.drop-target"), `${litCard()}`);
    await sleep(150);
    await api.capture("multi-card-lit", "multi-card-lit.png");
    await api.drag({ type: "over", position: await pointer(".pc-session") });
    await sleep(60);
    check("multi: vertical: off the cards the body as a whole is the target (current question)",
      litCard() === null && !!document.querySelector(".stage .pane-main.drop-target") && mainLabel() === "松手添加到第 2 题", `${litCard()} ${mainLabel()}`);
    await api.drag({ type: "drop", paths: [files[1]], position: await pointer(cardSel(0)) });
    check("multi: vertical: a drop on the first card goes to the first question", api.attachmentCount(2, 0) === 3 && api.attachmentCount(2, 1) === 3,
      `${api.attachmentCount(2, 0)}/${api.attachmentCount(2, 1)}`);
    await api.drag({ type: "enter", paths: files, position: await pointer(".pc-session") });
    await api.drag({ type: "drop", paths: [files[0]], position: await pointer(".pc-session") });
    check("multi: vertical: a drop off the cards goes to the question the request is on",
      api.attachmentCount(2, 0) === 4 && api.attachmentCount(2, 1) === 3, `${api.attachmentCount(2, 0)}/${api.attachmentCount(2, 1)}`);

    // The pointer rests just inside the bottom edge of the body: it scrolls down by itself, down to the last card.
    bodyEl().scrollTop = 0;
    await sleep(60);
    const bodyBox = api.rect(".pc-bodies")!;
    await api.drag({ type: "enter", paths: files, position: await pointerAt(bodyBox.left + bodyBox.width / 2, bodyBox.top + bodyBox.height - 12) });
    await sleep(500);
    check("multi: resting at the bottom edge scrolls the body down", bodyEl().scrollTop > 100, `${round(bodyEl().scrollTop)}`);
    check("multi: it reaches the end of the body", await until(() => bodyEl().scrollTop >= bodyEl().scrollHeight - bodyEl().clientHeight - 1, 6000), `${round(bodyEl().scrollTop)}`);
    await api.drag({ type: "over", position: await pointer(cardSel(2)) });
    await sleep(60);
    check("multi: the last question's card can be aimed at", litCard() === "2", `${litCard()}`);
    await sleep(150);
    await api.capture("multi-card-last", "multi-card-last.png");
    const resting = bodyEl().scrollTop;
    await sleep(300);
    check("multi: a pointer in the middle of the body scrolls nothing", near(bodyEl().scrollTop, resting, 1), `${round(resting)} → ${round(bodyEl().scrollTop)}`);
    await api.drag({ type: "drop", paths: files, position: await pointer(cardSel(2)) });
    check("multi: vertical: a drop on the last card goes to the third question", api.attachmentCount(2, 2) === 2 && api.currentQuestion(2) === 2,
      `${api.attachmentCount(2, 2)} current ${api.currentQuestion(2)}`);
    await sleep(100);
    const cardFiles = (i: number) => document.querySelectorAll(`.stage ${cardSel(i)} .pc-attachment`).length;
    check("multi: each card lists its own files", cardFiles(0) === 4 && cardFiles(1) === 3 && cardFiles(2) === 2, `${cardFiles(0)}/${cardFiles(1)}/${cardFiles(2)}`);
    // And back up again: the pointer at the top edge (the navigation bar above counts as the edge too).
    await api.drag({ type: "enter", paths: files, position: await pointerAt(bodyBox.left + bodyBox.width / 2, bodyBox.top + 12) });
    await sleep(500);
    check("multi: resting at the top edge scrolls the body up", bodyEl().scrollTop < resting - 100, `${round(resting)} → ${round(bodyEl().scrollTop)}`);
    check("multi: it reaches the start of the body", await until(() => bodyEl().scrollTop <= 1, 6000), `${round(bodyEl().scrollTop)}`);
    await api.drag({ type: "leave" });
    const leftAt = bodyEl().scrollTop;
    await sleep(200);
    check("multi: letting go stops the scrolling", near(bodyEl().scrollTop, leftAt, 1));
    await api.capture("multi-vertical", "multi-vertical.png");

    // The Sidebar list scrolls the same way while the pointer rests at its edge. Many requests make it overflow.
    api.tuning.questionLayout = "sequential";
    await api.perform({ type: "reset", count: 3 });
    for (let i = 0; i < 9; i++) await api.perform({ type: "arrive", kind: "ask", other: i % 2 === 0 });
    await sleep(500);
    const navEl = () => document.querySelector<HTMLElement>(".stage .inbox-navigation")!;
    check("multi: twelve requests overflow the Sidebar list", navEl().scrollHeight > navEl().clientHeight + 100, `${navEl().scrollHeight} vs ${navEl().clientHeight}`);
    navEl().scrollTop = 0;
    const navBox = api.rect(".inbox-navigation")!;
    await api.drag({ type: "enter", paths: files, position: await pointerAt(navBox.left + navBox.width / 2, navBox.top + navBox.height - 12) });
    await sleep(500);
    check("multi: resting at the bottom of the Sidebar list scrolls it", navEl().scrollTop > 100, `${round(navEl().scrollTop)}`);
    await api.drag({ type: "leave" });

    // The ready-made scene of the controller's "布置拖拽场景" button: the small window, three 稍后答 requests, the first with three questions.
    await api.perform({ type: "dragScene" });
    await settled("scene: laid out");
    snap = api.snapshot();
    check("scene: the drag scene rests as the small window with three 稍后答 requests",
      snap.view === "collapsed" && snap.requests.length === 3 && snap.requests.every(r => r.secondary), `${snap.view} ${snap.requests.length}`);
    await api.perform({ type: "expandDefault" });
    await settled("scene: peek");
    await sleep(300);
    snap = api.snapshot();
    check("scene: the first request is the one with three questions", snap.requests[0].title === "发布前的三个确认" && questionHead() === "第 1 / 3 题", `${questionHead()}`);
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });

    // ---- S15: leaving AskHuman for real — the app loses the activation like on a click in another app. It takes the
    // focus and hands it to Finder, so it only runs with `--focus`; the silent default never touches the user's focus.
    if (api.silent) {
      await step("S15 skipped (silent run)");
    } else {
      await step("S15");
      await api.perform({ type: "reset", count: 2 });
      api.click(flag(2));
      await sleep(150);
      api.click('[data-act="put-away"]');
      await settled("away: collapse");
      await api.perform({ type: "expandDefault" });
      await settled("away: peek");
      check("away: the peek shows only 稍后答 requests", api.snapshot().view === "expanded" && api.snapshot().requests.every(r => r.secondary));
      const accepted = await api.deactivate();
      check("away: macOS accepted handing the activation to another app", accepted);
      if (accepted) {
        check("away: the page learns that AskHuman is no longer active", await until(() => !api.appActive(), 4000));
        check("away: leaving for real shrinks the peek to the small window", await until(() => api.snapshot().view === "collapsed", 4000));
        await settled("away: shrunk");
        const gone = await info();
        check("away: the window is the small window", near(gone.width, api.mini().w) && near(gone.height, api.mini().h), rect(gone));
      }
      await api.activate();
      check("away: coming back reports AskHuman active again", await until(() => api.appActive(), 4000));
      await api.perform({ type: "arrive", kind: "ask" });
      await settled("away: newcomer");
      check("away: the newcomer expands the small window", api.snapshot().view === "expanded" && api.snapshot().requests.some(r => !r.secondary));
      if (await api.deactivate()) {
        await until(() => !api.appActive(), 4000);
        await sleep(700);
        check("away: leaving for real never shrinks a window that has a primary request", api.snapshot().view === "expanded");
      }
      await api.activate();
    }
  } catch (error) {
    check("scenario completes without error", false, String(error));
  }

  const failed = results.filter(r => !r.pass);
  await api.record({ label: "auto-result", pass: failed.length === 0, results }, true);
}
