const T0 = performance.now();
export type BootTier = 2000 | 5000 | 10000;

/** Run non-critical boot work in a stagger tier after app start, so first
 *  paint and the boot-critical fetches never compete with it. Returns a
 *  cancel function. */
export function afterBoot(tier: BootTier, task: () => void): () => void {
  const id = window.setTimeout(task, Math.max(0, tier - (performance.now() - T0)));
  return () => window.clearTimeout(id);
}
