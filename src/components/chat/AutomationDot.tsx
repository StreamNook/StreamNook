// The small dot beside a chat's name when an automation can act there: filled
// while rules are on, amber while StreamNook cannot be reached. Touch shows no
// tooltip, and a tap does nothing.
import { Tooltip } from '../ui/Tooltip';
import type { AutomationDotState } from '../../utils/automations';

export const AUTOMATION_DOT_WORDS: Record<AutomationDotState, string> = {
  on: 'Automations are on here',
  waiting: 'Automations are waiting for StreamNook',
};

export function AutomationDot({ state, onVideo = false }: { state: AutomationDotState; onVideo?: boolean }) {
  const words = AUTOMATION_DOT_WORDS[state];
  const look =
    state === 'on'
      ? onVideo
        ? 'bg-white shadow-[0_0_0_1px_rgba(0,0,0,0.4)]'
        : 'bg-accent'
      : 'bg-warning [html[data-theme-tone=light]_&]:bg-[color-mix(in_srgb,var(--color-warning)_58%,black)]';
  return (
    <Tooltip content={words}>
      <span role="img" aria-label={words} className="inline-flex h-4 w-3.5 shrink-0 items-center justify-center">
        <span className={`block h-1.5 w-1.5 rounded-full ${look}`} />
      </span>
    </Tooltip>
  );
}
