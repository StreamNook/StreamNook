// The selectors the cheap-draw caps leave alone (globals.css, Cheap draw: the
// :not() of both caps). loopCap.test.ts pins that this equals the CSS list.
// Loaders keep moving, emote modifiers are chat content, the offline drift is
// its own feature, and the one-shot cues keep their middle frames.
export const CHEAP_EXEMPT = [
  '.animate-spin',
  '.animate-penrose-rotate',
  '.pm-cube',
  '.animate-pulse:not(.sn-decor)',
  '.ghost-card',
  '.sn-listen-dots > i',
  '.sn-loader',
  '.sn-loader *',
  '[class*="sn-ffz-anim-"]',
  '[class*="sn-bttv-anim-"]',
  '.offline-bounce-x',
  '.offline-bounce-y',
  '.settings-row-found',
  '.animate-highlight-flash',
  '.animate-mention-flash',
  '.animate-reply-flash',
  '.animate-phrase-highlight',
  '.whisper-highlight',
  '.chat-message-row[data-sn-flash]',
  '.sn-mod-dwell',
  '.sn-mod-cap-dwell',
  '.animate-hype-marquee',
].join(', ');
