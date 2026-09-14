import { memo } from 'react';
import { Music } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
// Shared helper, not a local copy: this card renders inline in chat, so on
// Android its service links were dead for the same reason every other chat link
// was - `@tauri-apps/plugin-shell`'s open has no Android implementation.
import { openExternal } from '../../utils/openExternal';
import type { SongMatch } from '../../utils/songId';

// The "/song" result rendered inline in chat: cover art, title/artist, and a row
// of borderless service links (Spotify, Apple Music, ...) plus the song.link
// aggregator. Each link opens in the browser.
export const SongCard = memo(function SongCard({ card }: { card: SongMatch }) {
  const links = [...card.providers];
  if (card.song_link) links.push({ name: 'song.link', url: card.song_link });

  return (
    <div className="glass-panel rounded-lg p-2 flex items-start gap-3 max-w-[320px]">
      {card.album_art ? (
        <img
          src={card.album_art}
          alt=""
          className="w-14 h-14 rounded-md object-cover flex-shrink-0"
          loading="lazy"
        />
      ) : (
        <div className="w-14 h-14 rounded-md bg-white/5 flex items-center justify-center flex-shrink-0">
          <Music className="w-5 h-5 text-white/40" />
        </div>
      )}

      <div className="min-w-0 flex flex-col gap-0.5">
        <Tooltip content={card.title}>
          <div className="text-sm font-semibold text-white truncate">
            {card.title}
          </div>
        </Tooltip>
        <div className="text-xs text-white/60 truncate">
          {card.artist}
          {card.album ? ` · ${card.album}` : ''}
        </div>

        {links.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 mt-1.5">
            {links.map((l) => (
              <Tooltip key={l.url} content={l.url} side="top">
                <button
                  onClick={() => {
                    void openExternal(l.url);
                  }}
                  className="text-xs font-medium text-white/70 hover:text-accent transition-colors duration-150"
                >
                  {l.name}
                </button>
              </Tooltip>
            ))}
          </div>
        )}
      </div>
    </div>
  );
});
