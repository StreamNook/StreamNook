// The small dot beside a chat's name when an automation can act there: filled
// while rules are on, hollow while StreamNook cannot be reached. Touch shows no
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
    state === 'on' ? 'bg-accent' : onVideo ? 'border-[1.5px] border-white/80' : 'border-[1.5px] border-accent';
  return (
    <Tooltip content={words}>
      <span role="img" aria-label={words} className="inline-flex h-4 w-3.5 shrink-0 items-center justify-center">
        <span className={`block h-1.5 w-1.5 rounded-full ${look}`} />
      </span>
    </Tooltip>
  );
}
