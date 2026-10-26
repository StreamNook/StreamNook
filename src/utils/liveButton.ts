// The LIVE control both players inject into Plyr's bar: red at the live edge,
// grey when behind (buffering, paused, rewound, or a swap in flight), and the
// one click that gets back there. Its look lives in globals.css
// (`.plyr__control.sn-live-btn`); this is its markup and its painter.

export type LiveButtonState = 'live' | 'behind' | 'catching-up';

export const LIVE_BUTTON_TIPS: Record<LiveButtonState, string> = {
  live: 'Watching live',
  behind: 'Behind live · click to catch up',
  'catching-up': 'Catching up…',
};

/** Static markup for `injectPlyrControl`; the state is painted afterwards. */
export const LIVE_BUTTON_HTML =
  '<span class="sn-live-btn__dot" aria-hidden="true"></span>' +
  '<span class="sn-live-btn__label">LIVE</span>' +
  '<span class="plyr__tooltip" role="tooltip"></span>';

/** The state a freshly inserted LIVE control starts in: behind when the viewer
 *  is rewound or the video is paused (a paused player is never at the edge),
 *  otherwise whatever the page last knew. */
export function initialLiveButtonState(o: {
  rewound: boolean;
  paused: boolean;
  last: LiveButtonState;
}): LiveButtonState {
  return o.rewound || o.paused ? 'behind' : o.last;
}

/** Paints the injected LIVE control. Idempotent: returns at once when the
 *  state it shows already matches, so callers can be generous with it. */
export function paintLiveButton(btn: HTMLButtonElement, state: LiveButtonState): void {
  if (btn.dataset.state === state) return;
  btn.dataset.state = state;
  btn.setAttribute('aria-label', LIVE_BUTTON_TIPS[state]);
  const tip = btn.querySelector('.plyr__tooltip');
  if (tip) tip.textContent = LIVE_BUTTON_TIPS[state];
}
