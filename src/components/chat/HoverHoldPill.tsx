import type { Ref } from 'react';
import { Pause } from 'phosphor-react';
import { fitCount } from '../../utils/fitNumber';

/**
 * What a moderator's hover hold keeps back (ChatMessageList). A status, not a
 * button: it takes no pointer, and screen readers skip it (its count changes
 * with every row). It waits unseen until the list says it may show
 * (`shown`), and steps aside while the pointer is near it: the list writes
 * data-near on its node and globals.css hides it. It sits where Chat Paused
 * does. Moving off chat, or resting, lets chat go.
 */
export function HoverHoldPill({ count, shown, ref }: { count: number; shown: boolean; ref?: Ref<HTMLDivElement> }) {
  return (
    <div
      ref={ref}
      data-hold-pill="true"
      data-due={shown ? 'true' : undefined}
      aria-hidden="true"
      className="pointer-events-none absolute bottom-3 left-1/2 transform -translate-x-1/2 z-50"
    >
      <div className="sn-hold-pill flex items-center gap-2 whitespace-nowrap px-3.5 py-1.5 chrome-glaze chrome-glaze--frosted text-white text-[13px] font-medium shadow-lg">
        <Pause size={14} weight="fill" />
        <span>{`Paused on Hover (${fitCount(count)} new)`}</span>
      </div>
    </div>
  );
}
