import React from 'react';
import { inlineEmoteTier, sevenTvTierUrl } from '../../services/emoteService';
import type { EmoteTabCandidate } from '../../utils/chatInputWord';

type ThumbEmote = Pick<NonNullable<EmoteTabCandidate['emote']>, 'id' | 'name' | 'url' | 'localUrl' | 'provider'>;

/**
 * One emote image for the composer's completions. Disk first (`localUrl` is the
 * cached file at the per-DPI tier), CDN at that tier on a miss; a stale disk
 * file or a missing 7TV size walks the avif then webp ladder so the thumb is
 * never blank.
 */
const EmoteThumb: React.FC<{ emote: ThumbEmote; size: number }> = ({ emote, size }) => {
  const tier = inlineEmoteTier();
  const src = emote.provider === '7tv'
    ? (emote.localUrl || sevenTvTierUrl(emote.id, tier))
    : (emote.localUrl || emote.url);
  return (
    <img
      src={src}
      alt={emote.name}
      loading="lazy"
      draggable={false}
      style={{ maxHeight: size, maxWidth: size }}
      className="object-contain"
      onError={(e) => {
        const t = e.currentTarget;
        if (emote.provider === '7tv') {
          const ladder = [`${tier}.avif`, '2x.avif', '1x.avif', '2x.webp', '1x.webp']
            .map((s) => `https://cdn.7tv.app/emote/${emote.id}/${s}`);
          let step = Number(t.dataset.fb || '0');
          while (step < ladder.length && ladder[step] === t.src) step++;
          if (step < ladder.length) {
            t.dataset.fb = String(step + 1);
            t.src = ladder[step];
          }
          return;
        }
        if (emote.localUrl && t.src !== emote.url) t.src = emote.url;
      }}
    />
  );
};

export default EmoteThumb;
