export const arrivalTiming = { appear: 220, hold: 500, fly: 560, land: 640 };
export interface ArrivalPoint { x: number; y: number }
export interface ArrivalGeometry { origin: ArrivalPoint; target: ArrivalPoint }
export interface ArrivalFrame extends ArrivalPoint {
  scale: number; opacity: number; angle: number; morph: number; plateOpacity: number; haloOpacity: number; done: boolean;
}
const clamp = (n: number) => Math.max(0, Math.min(1, n));
const ease = (n: number) => { const t = clamp(n); return t * t * t * (t * (t * 6 - 15) + 10); };
const mix = (a: number, b: number, t: number) => a + (b - a) * t;

export function arrivalFrame(elapsed: number, { origin, target }: ArrivalGeometry): ArrivalFrame {
  const frame: ArrivalFrame = { ...origin, scale: 1, opacity: 1, angle: 0, morph: 0, plateOpacity: 1, haloOpacity: 1, done: false };
  if (elapsed < arrivalTiming.appear) {
    const t = ease(elapsed / arrivalTiming.appear);
    frame.scale = mix(.78, 1, t); frame.opacity = t; frame.y += mix(13, 0, t);
    return frame;
  }
  if (elapsed < arrivalTiming.appear + arrivalTiming.hold) return frame;
  const progress = clamp((elapsed - arrivalTiming.appear - arrivalTiming.hold) / arrivalTiming.fly);
  const t = ease(progress), u = 1 - t;
  const lift = Math.min(62, Math.abs(target.x - origin.x) * .22 + 22);
  const c1 = { x: mix(origin.x, target.x, .25), y: origin.y - lift };
  const c2 = { x: mix(target.x, origin.x, .25), y: target.y - Math.min(30, lift / 2) };
  frame.x = u ** 3 * origin.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t ** 3 * target.x;
  frame.y = u ** 3 * origin.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t ** 3 * target.y;
  // The Lucide circle has a 20-unit diameter in its 24-unit viewBox.
  frame.scale = mix(1, 10 / (88 * 20 / 24), t);
  frame.angle = -8 * Math.sin(Math.PI * t);
  frame.morph = ease((progress - .32) / .6);
  frame.plateOpacity = 1 - ease((progress - .4) / .45);
  frame.haloOpacity = 1 - ease(progress / .6);
  frame.done = progress === 1;
  return frame;
}

export type ArrivalEnd = "landed" | "cancelled" | "unavailable";
// Each request owns its epoch. Sharing a frame callback does not serialize arrivals.
export function arrivalAnimator(options: {
  measure: (id: string) => ArrivalGeometry | null;
  paint: (id: string, frame: ArrivalFrame) => void;
  settle: (id: string, reason: ArrivalEnd) => void;
  now?: () => number;
  schedule?: (callback: FrameRequestCallback) => number;
  cancel?: (frame: number) => void;
}) {
  const epochs = new Map<string, number>();
  const schedule = options.schedule ?? requestAnimationFrame;
  const cancel = options.cancel ?? cancelAnimationFrame;
  let frame: number | null = null;
  function finish(id: string, reason: ArrivalEnd) {
    if (!epochs.delete(id)) return;
    options.settle(id, reason);
    if (!epochs.size && frame !== null) { cancel(frame); frame = null; }
  }
  function tick(now: number) {
    frame = null;
    for (const [id, epoch] of [...epochs]) {
      if (!epochs.has(id)) continue;
      const geometry = options.measure(id);
      if (!geometry) { finish(id, "unavailable"); continue; }
      const state = arrivalFrame(now - epoch, geometry);
      options.paint(id, state);
      if (state.done) finish(id, "landed");
    }
    if (epochs.size) frame = schedule(tick);
  }
  return {
    play(id: string) {
      if (epochs.has(id)) return;
      epochs.set(id, (options.now ?? (() => performance.now()))());
      if (frame === null) frame = schedule(tick);
    },
    cancel: (id: string) => finish(id, "cancelled"),
    clear() { for (const id of [...epochs.keys()]) finish(id, "cancelled"); },
  };
}
