// The accepted Sidebar ripple preset. Feather widths are in screen pixels.
export const unreadRipple = {
  expandMs: 2000,
  fadeStartMs: 800,
  fadeMs: 1500,
  cycleMs: 3300,
  startRadiusPx: 5,
  coverage: 1.05,
  featherPx: 100,
} as const;

function expansion(time: number): number {
  if (time <= 0) return 0;
  if (time >= 1) return 1;
  // Invert the time axis of cubic-bezier(.684, 0, .74, 0).
  let low = 0;
  let high = 1;
  for (let i = 0; i < 16; i++) {
    const t = (low + high) / 2;
    const x = 3 * (1 - t) ** 2 * t * .684 + 3 * (1 - t) * t ** 2 * .74 + t ** 3;
    if (x < time) low = t;
    else high = t;
  }
  return ((low + high) / 2) ** 3;
}

export function rippleRadius(width: number, height: number, x: number, y: number): number {
  return Math.hypot(Math.max(x, width - x), Math.max(y, height - y)) * unreadRipple.coverage;
}

export function rippleFrame(elapsedMs: number, radius: number) {
  const time = Math.max(0, elapsedMs) % unreadRipple.cycleMs;
  const progress = expansion(time / unreadRipple.expandMs);
  const scale = unreadRipple.startRadiusPx / radius + (1 - unreadRipple.startRadiusPx / radius) * progress;
  const opacity = Math.max(0, Math.min(1, 1 - (time - unreadRipple.fadeStartMs) / unreadRipple.fadeMs));
  // Compensate for scale so feathering grows in physical row coordinates.
  const feather = unreadRipple.featherPx * progress ** 2 / scale;
  return {
    scale,
    opacity,
    opaquePercent: Math.max(0, 100 * (1 - feather / radius)),
    sleepMs: opacity === 0 ? unreadRipple.cycleMs - time : 0,
  };
}
