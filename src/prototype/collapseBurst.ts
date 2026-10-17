// Exploratory frame capture (`scripts/popup-collapse-demo.sh --burst`): while the window collapses and expands, a
// native background thread records what is really composited and counts "torn" frames (the window's right or
// bottom edge see-through because the content layer already has the new smaller size while the window server still
// shows the old frame). Used to compare ways of handing frames to the window server; no pass / fail.
// Torn frames are saved as PNGs in `.askhuman-dev/popup-collapse-shots/<name>/`.
import { DEFAULT_TUNING } from "./collapseShared";
import type { StageApi } from "./collapseAuto";

const sleep = (ms: number) => new Promise<void>(resolve => setTimeout(resolve, ms));

async function until(condition: () => boolean, timeoutMs = 8000) {
  const start = performance.now();
  while (!condition()) {
    if (performance.now() - start > timeoutMs) return false;
    await sleep(16);
  }
  return true;
}

interface Variant { name: string; sync: string | undefined; pauseUs: number }

const ALL_VARIANTS: Variant[] = [
  { name: "baseline/hammer", sync: undefined, pauseUs: 0 },
  { name: "baseline/gentle", sync: undefined, pauseUs: 12000 },
  { name: "display/hammer", sync: "display", pauseUs: 0 },
  { name: "display/gentle", sync: "display", pauseUs: 12000 },
  { name: "screens/hammer", sync: "screens", pauseUs: 0 },
  { name: "screens/gentle", sync: "screens", pauseUs: 12000 },
  { name: "both/hammer", sync: "both", pauseUs: 0 },
  { name: "both/gentle", sync: "both", pauseUs: 12000 },
  { name: "group/hammer", sync: "group", pauseUs: 0 },
  { name: "group/gentle", sync: "group", pauseUs: 12000 },
  { name: "group+screens/hammer", sync: "group+screens", pauseUs: 0 },
  { name: "group+screens/gentle", sync: "group+screens", pauseUs: 12000 },
];

export async function runBurst(api: StageApi) {
  const summary: string[] = [];
  const query = new URLSearchParams(location.search);
  const cycles = Number(query.get("cycles") ?? 8);
  const only = query.get("variants")?.split(",");
  const variants = only ? ALL_VARIANTS.filter(v => only.some(o => v.name.startsWith(o))) : ALL_VARIANTS;
  try {
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });
    await sleep(300);
    await api.perform({ type: "reset", count: 3 });
    const flag = (id: number) => `.inbox-entry:has([data-inbox-row="${id}"]) .row-flag`;
    // Everything but the open request is secondary, so one footer click collapses the window.
    api.click(flag(2));
    api.click(flag(3));
    await sleep(300);

    const settle = async (view: "collapsed" | "expanded") => {
      await until(() => api.snapshot().phase === "idle" && api.snapshot().view === view);
      await sleep(250);
    };
    // From here on every cycle is "leave AskHuman" (collapse) → "click the small window" (expand), on a 稍后答 request.
    api.click('[data-act="put-away"]');
    await settle("collapsed");
    await api.perform({ type: "expandDefault" });
    await settle("expanded");
    for (const variant of variants) {
      api.setSync(variant.sync);
      const slug = variant.name.replace("/", "-");
      const totals = { collapse: { cycles: 0, torn: 0, tears: 0, dropped: 0 }, expand: { cycles: 0, torn: 0, tears: 0, dropped: 0 } };
      for (let cycle = 1; cycle <= cycles; cycle++) {
        for (const kind of ["collapse", "expand"] as const) {
          await api.burstStart(`${kind}-${slug}-${cycle}`, 600, variant.pauseUs, false);
          await sleep(150);
          // The window the cycle expands shows a "稍后答" request: leaving AskHuman (C23) is what collapses it.
          if (kind === "collapse") await api.perform({ type: "leave" });
          else await api.perform({ type: "expandDefault" });
          await settle(kind === "collapse" ? "collapsed" : "expanded");
          const { tears } = await api.burstStop();
          const bucket = totals[kind];
          bucket.cycles++;
          bucket.tears += tears;
          if (tears > 0) bucket.torn++;
          bucket.dropped += api.lastReport()?.report.dropped ?? 0;
        }
      }
      const line = (kind: "collapse" | "expand") => {
        const t = totals[kind];
        return `${kind} ${t.torn}/${t.cycles} cycles torn, ${t.tears} torn frames, ${t.dropped} dropped ticks`;
      };
      summary.push(`${variant.name.padEnd(22)} ${line("collapse")}  |  ${line("expand")}`);
    }
    api.setSync(undefined);
    await api.record({ label: "burst-result", pass: true, summary }, true);
  } catch (error) {
    await api.record({ label: "burst-result", pass: false, error: String(error), summary }, true);
  }
}

/** Screen region (points, top-left origin on the main display) the outside recorder captures; see the script. */
const VIDEO_REGION: [number, number, number, number] = [400, 60, 1000, 760];

type DebugFlags = Partial<{ noMotion: boolean; noAnim: boolean; hidden: boolean; shadow: boolean | undefined }>;
/** Isolation variants of the video run: what is switched off to find the source of a frame hitch. */
const VIDEO_VARIANTS: Record<string, DebugFlags> = {
  base: { noMotion: false, noAnim: false, hidden: false, shadow: true },
  noshadow: { noMotion: false, noAnim: false, hidden: false, shadow: false },
  nomotion: { noMotion: true, noAnim: false, hidden: false, shadow: true },
  noanim: { noMotion: false, noAnim: true, hidden: false, shadow: true },
  still: { noMotion: true, noAnim: true, hidden: false, shadow: true },
  hidden: { noMotion: false, noAnim: false, hidden: true, shadow: true },
  bare: { noMotion: true, noAnim: true, hidden: true, shadow: false },
};

/**
 * Collapse / expand cycles on a plain backdrop. `scripts/popup-collapse-demo.sh --video` records the screen region
 * from outside this process (starting when the "video-ready" record appears); the clip then shows the real display
 * output frame by frame. `variants=base,noshadow,…` (ASKHUMAN_DEMO_QUERY) runs `cycles` cycles per variant in order.
 * No pass / fail here — the clip is analysed offline.
 */
export async function runVideo(api: StageApi) {
  const query = new URLSearchParams(location.search);
  const cycles = Number(query.get("cycles") ?? 24);
  const rest = Number(query.get("rest") ?? 700);
  // `rounds` repeats the whole variant list so that slow changes of the machine's load hit every variant alike.
  const rounds = Number(query.get("rounds") ?? 1);
  const names = (query.get("variants") ?? "base").split(",").filter(name => name in VIDEO_VARIANTS || name.startsWith("ctl-"));
  try {
    Object.assign(api.tuning, DEFAULT_TUNING, { slow: 1 });
    await sleep(300);
    await api.perform({ type: "reset", count: 3 });
    const flag = (id: number) => `.inbox-entry:has([data-inbox-row="${id}"]) .row-flag`;
    api.click(flag(2));
    api.click(flag(3));
    await sleep(300);
    const settle = async (view: "collapsed" | "expanded") => {
      await until(() => api.snapshot().phase === "idle" && api.snapshot().view === view);
    };
    // From here on every cycle is "leave AskHuman" (collapse) → "click the small window" (expand), on a 稍后答 request.
    api.click('[data-act="put-away"]');
    await settle("collapsed");
    await api.perform({ type: "expandDefault" });
    await settle("expanded");
    await sleep(300);
    await api.backdrop(true, VIDEO_REGION);
    await sleep(500);
    await api.record({ label: "video-ready", region: VIDEO_REGION, variants: names, cycles }, false);
    await sleep(4000);

    for (let round = 0; round < rounds; round++) {
      for (const name of names) {
        if (name.startsWith("ctl-")) {
          // Reference animation on a plain window instead of the answer window (sizes: expanded + sidebar -> mini).
          await api.record({ label: "video-variant", name, cycles }, false);
          await api.control(name.slice(4), cycles, rest, [826, 652, 260, 256]);
          continue;
        }
        api.setDebug(VIDEO_VARIANTS[name]);
        await api.record({ label: "video-variant", name, cycles }, false);
        for (let cycle = 1; cycle <= cycles; cycle++) {
          await api.perform({ type: "leave" });
          await settle("collapsed");
          await sleep(rest);
          await api.perform({ type: "expandDefault" });
          await settle("expanded");
          await sleep(rest);
        }
      }
    }
    api.setDebug(VIDEO_VARIANTS.base);
    await api.backdrop(false, VIDEO_REGION);
    await api.record({ label: "video-result", pass: true, cycles, rounds, variants: names }, true);
  } catch (error) {
    await api.backdrop(false, VIDEO_REGION);
    await api.record({ label: "video-result", pass: false, error: String(error) }, true);
  }
}
