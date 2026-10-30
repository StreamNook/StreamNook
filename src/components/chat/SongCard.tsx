import { memo, useSyncExternalStore } from 'react';
import { ListMusic, Music } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
// The one song link opener: the music app on this computer when one is
// installed, else the browser; on the phones the system's own opener (the
// shell plugin's open has no Android implementation).
import { openSongLink } from '../../utils/songLinks';
import type { SongMatch } from '../../utils/songId';
import { hasSongsSurface, openSongs, subscribeSongsSurface } from '../../utils/songHistory';
import { cardLinks } from '../songs/songLinks';

const noSurface = () => false;

// The "/song" result rendered inline in chat: cover art, title/artist, and a row
// of service links plus the song.link aggregator. The track's own links come
// first; search pages show only when it has none, marked as searches. Each
// link opens in its music app when one is installed, else in the browser
// (utils/songLinks.ts). A quiet button at the end of the title opens the
// song in Your songs, where it is kept, in windows that can show that list (it
// appears once that list's surface is mounted).
export const SongCard = memo(function SongCard({ card }: { card: SongMatch }) {
  const { links, search } = cardLinks(card);
  const canOpen = useSyncExternalStore(subscribeSongsSurface, hasSongsSurface, noSurface);
  const kept = card.key && canOpen ? card.key : null;

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

      <div className="min-w-0 flex-1 flex flex-col gap-0.5 -mt-0.5">
        <div className="flex items-center gap-1.5">
          <Tooltip content={card.title}>
            <div className="min-w-0 truncate text-sm font-semibold text-textPrimary">
              {card.title}
            </div>
          </Tooltip>
          {kept && (
            <Tooltip content="Open in Your songs">
              <button
                type="button"
                aria-label="Open in Your songs"
                onClick={() => openSongs(kept)}
                className="ml-auto -my-0.5 flex h-5 w-5 shrink-0 items-center justify-center text-textMuted transition-colors hover:text-textPrimary"
              >
                <ListMusic size={14} />
              </button>
            </Tooltip>
          )}
        </div>
        <div className="text-xs text-textSecondary truncate">
          {card.artist}
          {card.album ? ` · ${card.album}` : ''}
        </div>

        {links.length > 0 && (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-0.5 mt-1.5">
            {search && <span className="text-xs text-textMuted">Search on</span>}
            {links.map((l) => (
              <Tooltip key={l.url} content={l.url} side="top">
                <button
                  onClick={() => {
                    void openSongLink(l.url);
                  }}
                  className="text-xs font-medium text-textSecondary hover:text-accent transition-colors duration-150"
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
