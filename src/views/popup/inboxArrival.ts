export const arrivalTiming = { appear: 220, first: 1000, second: 616, fly: 750, land: 640 };
const motion = { firstHeight: 52, secondHeight: 32, secondDuration: 880, squash: .07, hook: 84 };
export interface ArrivalPoint { x: number; y: number }
export interface ArrivalGeometry { origin: ArrivalPoint; target: ArrivalPoint }
export interface ArrivalFrame extends ArrivalPoint {
  scale: number; stretchX: number; stretchY: number; opacity: number; angle: number; morph: number; plateOpacity: number; haloOpacity: number; done: boolean;
}
const clamp = (n: number) => Math.max(0, Math.min(1, n));
const ease = (n: number) => { const t = clamp(n); return t * t * t * (t * (t * 6 - 15) + 10); };
const mix = (a: number, b: number, t: number) => a + (b - a) * t;
const flightAt = arrivalTiming.appear + arrivalTiming.first + arrivalTiming.second;

function shape(points: readonly (readonly [number, number])[], at: number) {
  if (at <= points[0][0]) return points[0][1];
  if (at >= points[points.length - 1][0]) return points[points.length - 1][1];
  let i = 0; while (points[i + 1][0] < at) i++;
  const a = points[i], b = points[i + 1], dt = b[0] - a[0], t = (at - a[0]) / dt;
  const slope = (j: number) => j === 0 || j === points.length - 1 ? 0 : (points[j + 1][1] - points[j - 1][1]) / (points[j + 1][0] - points[j - 1][0]);
  return (2*t*t*t-3*t*t+1)*a[1] + (t*t*t-2*t*t+t)*dt*slope(i) + (-2*t*t*t+3*t*t)*b[1] + (t*t*t-t*t)*dt*slope(i+1);
}
function contact(p: number, startVelocity: number, endVelocity: number, duration: number) {
  return (p*p*p-2*p*p+p)*duration*startVelocity + (p*p*p-p*p)*duration*endVelocity;
}
function hop(t: number, height: number, second: boolean) {
  const start = second ? 0 : .06, end = second ? .90 : .92, air = end - start;
  const velocity = 4 * height / air;
  if (t < start) return contact(t / start, 0, -velocity, start);
  if (t <= end) { const p = (t - start) / air; return -4 * height * p * (1 - p); }
  const nextVelocity = -4 * motion.secondHeight / .90 * arrivalTiming.first / motion.secondDuration;
  return contact((t - end) / (1 - end), velocity, nextVelocity, 1 - end);
}

function flightPath(origin: ArrivalPoint, target: ArrivalPoint) {
  const p = arrivalTiming.second / motion.secondDuration / .90;
  const start = { x: origin.x, y: origin.y - 4 * motion.secondHeight * p * (1 - p) };
  const vy = 4 * motion.secondHeight * (2 * p - 1) / (.90 * motion.secondDuration);
  // Descend vertically, curl around the bottom of the hook, then sweep toward the live dot.
  const q = .32, u = 1 - q;
  const c2 = { x: mix(target.x, start.x, .7), y: target.y + Math.min(24, motion.hook * .2) };
  const c1 = { x: start.x, y: (start.y + motion.hook - u ** 3 * start.y - 3*u*q*q*c2.y - q ** 3 * target.y) / (3*u*u*q) };
  // Lower sidebar rows must still begin with downward motion rather than reversing the hook.
  c1.y = Math.max(c1.y, start.y + motion.hook);
  const dy = 3 * (c1.y - start.y), ddy = 6 * (c2.y - 2*c1.y + start.y);
  const rate = vy * arrivalTiming.fly / dy;
  const incomingAcceleration = 8 * motion.secondHeight / (.90 ** 2 * motion.secondDuration ** 2);
  const acceleration = Math.max(0, Math.min(1.2, (incomingAcceleration * arrivalTiming.fly ** 2 - ddy * rate ** 2) / dy));
  return { start, c1, c2, rate, acceleration };
}

export function arrivalFrame(elapsed: number, { origin, target }: ArrivalGeometry): ArrivalFrame {
  const frame: ArrivalFrame = { ...origin, scale: 1, stretchX: 1, stretchY: 1, opacity: 1, angle: 0, morph: 0, plateOpacity: 1, haloOpacity: 1, done: false };
  if (elapsed < arrivalTiming.appear) {
    const t = ease(elapsed / arrivalTiming.appear);
    frame.scale = mix(.78, 1, t); frame.opacity = t; frame.y += mix(13, 0, t);
    return frame;
  }
  if (elapsed < flightAt) {
    const second = elapsed >= arrivalTiming.appear + arrivalTiming.first;
    const duration = second ? motion.secondDuration : arrivalTiming.first;
    const t = (elapsed - arrivalTiming.appear - (second ? arrivalTiming.first : 0)) / duration;
    const squash = motion.squash * (second ? .72 : 1);
    frame.y += hop(t, second ? motion.secondHeight : motion.firstHeight, second);
    frame.stretchY = second
      ? 1 + squash * .3 * Math.sin(Math.PI * clamp(t / .34)) ** 2
      : shape([[0,1],[.03,1-squash*.6],[.08,1+squash*.3],[.28,1],[.50,1],[.86,1+squash*.15],[.96,1-squash],[1,1]], t);
    frame.stretchX = 1 / frame.stretchY;
    return frame;
  }
  const progress = clamp((elapsed - flightAt) / arrivalTiming.fly);
  const { start, c1, c2, rate: a, acceleration: b } = flightPath(origin, target);
  // Carry the falling velocity and acceleration into flight without a stop or lateral snap.
  const t = progress === 1 ? 1 : a*progress + b/2*progress**2 + (10-6*a-1.5*b)*progress**3 + (-15+8*a+1.5*b)*progress**4 + (6-3*a-.5*b)*progress**5, u = 1 - t;
  frame.x = u ** 3 * start.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t ** 3 * target.x;
  frame.y = u ** 3 * start.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t ** 3 * target.y;
  // The Lucide circle has a 20-unit diameter in its 24-unit viewBox.
  frame.scale = mix(1, 10 / (88 * 20 / 24), ease(t));
  frame.angle = -8 * Math.sin(Math.PI * ease(progress));
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
