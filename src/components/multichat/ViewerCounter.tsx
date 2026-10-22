import { useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { UsersThree } from 'phosphor-react';
import { ProviderLogo } from '../ProviderLogo';
import { totalViewers, type SourceLive } from '../../utils/tabLive';

// A clean, aggregate viewer counter for the MultiChat title bar: total live
// viewers across every open source, with a per-stream breakdown on hover. It
// reads the window's shared live state (useTabLive), the same rows the
// tab strip's live dots read, so it asks nothing of any platform itself.

export default function ViewerCounter({ stats }: { stats: SourceLive[] }) {
  const [anchor, setAnchor] = useState<{ top: number; left: number } | null>(null);

  const total = useMemo(() => totalViewers(stats), [stats]);
  const liveCount = stats.filter((s) => s.isLive).length;

  if (stats.length === 0) return null;

  return (
    <>
      <button
        type="button"
        data-tauri-drag-region="false"
        onMouseEnter={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setAnchor({ top: r.bottom + 6, left: r.left });
        }}
        onMouseLeave={() => setAnchor(null)}
        className="flex items-center gap-1.5 rounded px-2 py-0.5 text-xs text-textSecondary transition-colors hover:bg-white/5 hover:text-textPrimary"
      >
        <UsersThree size={13} weight="fill" />
        <span className="font-medium tabular-nums">{total.toLocaleString()}</span>
        {liveCount > 0 && (
          <span className="relative flex h-1.5 w-1.5">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-red-500 opacity-60" />
            <span className="relative inline-flex h-1.5 w-1.5 rounded-full bg-red-500" />
          </span>
        )}
      </button>
      {anchor &&
        createPortal(
          <div
            className="glass-flyout no-live-blur fixed z-[300] min-w-[190px] rounded-xl"
            // No live blur: over the scrolling chat a live backdrop-blur flickers
            // in WebView2. The flyout's base is near-opaque on its own.
            style={{ top: anchor.top, left: anchor.left, backdropFilter: 'none', WebkitBackdropFilter: 'none' }}
          >
            <div className="px-2 pb-1 pt-0.5 text-[11px] font-semibold uppercase tracking-wider text-white/50">
              Viewers{liveCount > 0 ? ` · ${total.toLocaleString()} watching` : ''}
            </div>
            {stats.map((s) => (
              <div key={s.key} className="flex items-center gap-2 rounded-lg px-2 py-2 text-xs">
                <ProviderLogo provider={s.provider} size={13} />
                <span className="min-w-0 flex-1 truncate text-textSecondary">{s.name}</span>
                {s.isLive ? (
                  <span className="font-medium tabular-nums text-textPrimary">
                    {(s.count ?? 0).toLocaleString()}
                  </span>
                ) : (
                  <span className="text-[10px] uppercase tracking-wide text-textMuted">offline</span>
                )}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}
