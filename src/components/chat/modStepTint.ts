import type { ModStep } from '../../services/moderation';

/** A mod step's color, shared by the slider's track and the line buttons:
 *  Delete is a calm slate, timeouts warm from amber toward red as they
 *  lengthen, Ban is red. */
export function stepTint(steps: ModStep[], index: number): string {
  const step = steps[index];
  if (step.action === 'ban') return 'var(--color-error)';
  if (step.action === 'delete') return 'color-mix(in srgb, var(--color-text-secondary) 55%, #334155)';
  const timeouts = steps.filter((s) => s.action === 'timeout');
  const at = timeouts.indexOf(step);
  const toward = timeouts.length > 1 ? Math.round((at / (timeouts.length - 1)) * 65) : 0;
  return `color-mix(in srgb, var(--color-error) ${toward}%, var(--color-warning))`;
}
