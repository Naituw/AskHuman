export type CompletionFeedback = "sent" | "submitted";

export const answerTransition = { exit: 126, enter: 174, queue: 300 } as const;

/** The state of one answer transition, as the page and the Sidebar rows read it. */
export interface InboxMotion {
  from: string;
  completion: CompletionFeedback;
  phase: "waiting" | "out" | "prepare" | "in";
  incoming: string | null;
  reduced: boolean;
  rowHeight: number;
  group: string | null;
  groupHeight: number;
}

// Every awaited delay settles on disposal, including readiness deadlines.
export function transitionClock() {
  let cancelled = false;
  const pending = new Set<() => void>();
  return {
    get cancelled() { return cancelled; },
    onCancel(callback: () => void) {
      if (cancelled) callback(); else pending.add(callback);
      return () => { pending.delete(callback); };
    },
    wait(ms: number): Promise<boolean> {
      if (cancelled) return Promise.resolve(false);
      return new Promise(resolve => {
        const finish = (completed: boolean) => {
          clearTimeout(timer); pending.delete(cancel); resolve(completed);
        };
        const cancel = () => finish(false);
        const timer = setTimeout(() => finish(true), ms);
        pending.add(cancel);
      });
    },
    cancel() { cancelled = true; [...pending].forEach(cancel => cancel()); },
  };
}
