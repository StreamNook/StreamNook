// The card an emote opens when clicked in chat: the emote large, its name and
// where it comes from, and the people who made it as chips that go to their
// profiles (a Twitch account opens StreamNook's profile view; 7TV, BetterTTV
// and FFZ creators their page there). Rust answers who made it and builds the
// links (`emote_credit` with `detail`), cached per emote.
//
// 7TV emotes clicked in the main window open the 7TV spotlight instead, which
// carries the same creator chip plus its add-to-channel tools.
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion } from 'framer-motion';
import { Copy, ExternalLink, X } from 'lucide-react';

import { CreatorChip } from './chat/CreatorChip';
import { useAppStore } from '../stores/AppStore';
import { useEmoteCardStore, type EmoteCardTarget } from '../stores/emoteCardStore';
import {
  creatorRoleLabel,
  emoteLargeUrl,
  emoteProviderLabel,
  fetchEmoteCredit,
  type EmoteCredit,
} from '../services/emoteCredit';
import { openExternal } from '../utils/openExternal';

/** Art box: the emote large enough to read, wide emotes keep their shape. */
const ART_PX = 112;
/** How long the card waits for Rust before showing, so a normal answer lands
 *  before the first frame and the chips never push the card around. */
const DETAIL_WAIT_MS = 450;

export default function EmoteCard() {
  const emote = useEmoteCardStore((s) => s.emote);
  const close = useEmoteCardStore((s) => s.close);
  if (!emote) return null;
  return <Card key={`${emote.provider}:${emote.id}`} emote={emote} onClose={close} />;
}

function Card({ emote, onClose }: { emote: EmoteCardTarget; onClose: () => void }) {
  const [credit, setCredit] = useState<EmoteCredit | null>(null);
  const [settled, setSettled] = useState(false);
  const [lookupDone, setLookupDone] = useState(false);
  const [art, setArt] = useState(() => emoteLargeUrl(emote.provider, emote.id, emote.url));

  useEffect(() => {
    let live = true;
    const timer = window.setTimeout(() => live && setSettled(true), DETAIL_WAIT_MS);
    void fetchEmoteCredit({
      provider: emote.provider,
      id: emote.id,
      name: emote.name,
      channel: emote.channelLogin,
      channelId: emote.channelId,
      detail: true,
    }).then((c) => {
      if (!live) return;
      setCredit(c);
      setLookupDone(true);
      setSettled(true);
    });
    return () => {
      live = false;
      window.clearTimeout(timer);
    };
  }, [emote]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  }, [onClose]);

  if (!settled) return null;

  const provider = emoteProviderLabel(emote.provider);
  const subtitle = [
    credit?.originalName ? `Alias of ${credit.originalName}` : null,
    credit?.kindLabel ? `${provider} ${credit.kindLabel.replace(/^(\w)/, (c) => c.toLowerCase())}` : provider,
  ]
    .filter(Boolean)
    .join(' · ');
  const creators = credit?.creators ?? [];
  const pageUrl = credit?.pageUrl;

  const copyName = () => {
    void navigator.clipboard.writeText(emote.name).then(
      () => useAppStore.getState().addToast(`Copied ${emote.name}`, 'success'),
      () => useAppStore.getState().addToast('Could not copy the name', 'error'),
    );
  };

  return createPortal(
    <div className="fixed inset-0 z-[300] flex items-center justify-center bg-black/40 backdrop-blur-2xl">
      <div className="absolute inset-0" onClick={onClose} />
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label={emote.name}
        initial={{ opacity: 0, scale: 0.96, y: 12 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        transition={{ type: 'spring', stiffness: 360, damping: 26 }}
        className="liquid-glass-panel custom-scrollbar relative z-10 flex max-h-[86vh] w-[320px] max-w-[92vw] flex-col items-center overflow-y-auto px-6 pb-5 pt-6 text-center"
      >
        <button
          type="button"
          onClick={onClose}
          aria-label="Close"
          autoFocus
          className="absolute right-3 top-3 rounded p-1.5 text-textSecondary hover:bg-glass hover:text-textPrimary focus:outline-none focus-visible:ring-1 focus-visible:ring-accent/70"
        >
          <X size={16} />
        </button>

        <span className="flex items-center justify-center" style={{ height: ART_PX, maxWidth: '100%' }}>
          <img
            src={art}
            alt={emote.name}
            draggable={false}
            referrerPolicy="no-referrer"
            className="max-h-full max-w-full object-contain drop-shadow-md"
            onError={() => {
              if (art !== emote.url) setArt(emote.url);
            }}
          />
        </span>

        <div className="mt-3 flex flex-col items-center gap-0.5">
          <h2 className="text-lg font-semibold leading-tight text-textPrimary [overflow-wrap:anywhere]">{emote.name}</h2>
          <span className="text-xs text-textSecondary">{subtitle}</span>
        </div>

        {creators.length > 0 ? (
          <div className="mt-4 flex w-full flex-col gap-2.5 text-left">
            {creators.map((c) => (
              <div key={`${c.role}:${c.name}`} className="flex flex-col gap-1">
                <span className="px-0.5 text-[11px] text-textMuted">{creatorRoleLabel(c.role)}</span>
                <CreatorChip creator={c} onLeave={onClose} />
              </div>
            ))}
          </div>
        ) : (
          <p className="mt-3 text-xs text-textMuted">
            {lookupDone ? `${provider} doesn't list who made this one.` : 'Looking up who made it...'}
          </p>
        )}

        <div className="mt-4 flex items-center justify-center gap-4">
          {pageUrl && (
            <button
              type="button"
              onClick={() => void openExternal(pageUrl)}
              className="flex items-center gap-1.5 py-1 text-xs text-textSecondary hover:text-textPrimary"
            >
              <ExternalLink size={13} />
              Open on {provider}
            </button>
          )}
          <button
            type="button"
            onClick={copyName}
            className="flex items-center gap-1.5 py-1 text-xs text-textSecondary hover:text-textPrimary"
          >
            <Copy size={13} />
            Copy name
          </button>
        </div>
      </motion.div>
    </div>,
    document.body,
  );
}
