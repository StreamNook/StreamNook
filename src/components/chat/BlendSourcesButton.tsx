import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { CaretDown, Plus } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';
import { Toggle } from '../ui/Toggle';
import { ProviderMark } from '../ProviderLogo';
import { PROVIDERS, type ProviderId } from '../../types/providers';
import type { BlendCompanion, LinkSuggestion } from '../../hooks/useBlendCompanions';

// The combined chat's one control in the chat header. A mark per linked
// platform took about 22px each, and with three the header truncated its own
// title to "Combi...", so the platforms now overlap like an avatar group in one
// fixed-size capsule and the per-platform switches live in its menu. Everything
// here presents: show/hide, link and suggestions call back into ChatWidget,
// which writes the link records in Rust as before.

export interface BlendSourceRow {
  source: BlendCompanion;
  /** In this streamer's feed right now. */
  included: boolean;
  /** Turned off for every channel in Settings; the switch is inert here. */
  offInSettings: boolean;
  /** Why the source is not delivering, when it is not. */
  error?: string;
}

export function BlendSourcesButton({
  rows,
  suggestions,
  suggestionsUnseen,
  panelOpen,
  onToggle,
  onOpenSuggestions,
  onOpenEditor,
  onClosePanel,
}: {
  rows: BlendSourceRow[];
  suggestions: LinkSuggestion[];
  suggestionsUnseen: boolean;
  /** The link editor or suggestions bar under the header is showing. */
  panelOpen: boolean;
  onToggle: (source: BlendCompanion) => void;
  onOpenSuggestions: () => void;
  onOpenEditor: () => void;
  onClosePanel: () => void;
}) {
  const [menuOpen, setMenuOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [place, setPlace] = useState<{ left: number; top: number } | null>(null);

  // Under the button, kept inside the window. Measured as it opens.
  useLayoutEffect(() => {
    if (!menuOpen || !buttonRef.current) return;
    const r = buttonRef.current.getBoundingClientRect();
    const width = 248;
    setPlace({ left: Math.max(8, Math.min(r.left, window.innerWidth - width - 8)), top: r.bottom + 6 });
  }, [menuOpen]);

  // Escape or a click outside the button and the menu closes it.
  useEffect(() => {
    if (!menuOpen) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setMenuOpen(false);
    };
    const onDown = (e: PointerEvent) => {
      const t = e.target as Node;
      if (menuRef.current?.contains(t) || buttonRef.current?.contains(t)) return;
      setMenuOpen(false);
    };
    window.addEventListener('keydown', onKey);
    document.addEventListener('pointerdown', onDown, true);
    return () => {
      window.removeEventListener('keydown', onKey);
      document.removeEventListener('pointerdown', onDown, true);
    };
  }, [menuOpen]);

  const found = suggestions.map((s) => s.candidate.provider);
  const label = rows.length
    ? `Combined chat: ${rows.map((r) => PROVIDERS[r.source.provider].label).join(', ')}`
    : found.length
      ? `Also streaming on ${found.map((p) => PROVIDERS[p].label).join(' and ')}? Take a look`
      : 'Combine chat from another platform';

  const onClick = () => {
    // The bar under the header answers first: the same button puts it away.
    if (panelOpen) {
      onClosePanel();
      return;
    }
    // Nothing linked yet: straight to what there is to do.
    if (!rows.length) {
      if (found.length) onOpenSuggestions();
      else onOpenEditor();
      return;
    }
    setMenuOpen((o) => !o);
  };

  const marks: ProviderId[] = rows.length ? rows.map((r) => r.source.provider) : found;
  const dim = (p: ProviderId) => rows.length > 0 && !rows.find((r) => r.source.provider === p)?.included;

  return (
    <>
      <Tooltip content={menuOpen || panelOpen ? 'Close' : label} side="bottom">
        <button
          ref={buttonRef}
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onClick();
          }}
          aria-label={label}
          aria-haspopup={rows.length ? 'menu' : undefined}
          aria-expanded={menuOpen || panelOpen}
          className={`chrome-glaze chrome-glaze--flat chat-header-capsule blend-sources pointer-events-auto shrink-0 ${
            menuOpen || panelOpen ? 'is-open' : ''
          }`}
        >
          <span className="blend-sources-marks">
            {marks.length === 0 && <Plus size={10} weight="bold" className="text-textSecondary" />}
            {marks.map((p) => (
              <span key={p} className={`blend-sources-mark ${dim(p) ? 'is-off' : ''}`}>
                <ProviderMark provider={p} size={10} />
              </span>
            ))}
          </span>
          {rows.length > 0 && <CaretDown size={9} weight="bold" className="mr-0.5 text-textMuted" aria-hidden />}
          {suggestionsUnseen && !panelOpen && <span className="blend-trigger-dot" aria-hidden />}
        </button>
      </Tooltip>
      {menuOpen &&
        place &&
        createPortal(
          // Out of the chat header, whose stacking would put it under the
          // pinned message and polls, and whose overflow would clip it.
          <div
            ref={menuRef}
            role="menu"
            aria-label="Combined chat sources"
            className="glass-flyout fixed z-[70] w-[248px] rounded-xl p-1.5"
            style={{ left: place.left, top: place.top }}
          >
            <div className="px-2 pb-1 pt-0.5 text-[10px] font-semibold uppercase tracking-wider text-textMuted">
              In this chat
            </div>
            {rows.map((r) => {
              const meta = PROVIDERS[r.source.provider];
              return (
                <div
                  key={`${r.source.provider}:${r.source.channel}`}
                  role="menuitemcheckbox"
                  aria-checked={r.included}
                  aria-disabled={r.offInSettings}
                  className={`flex items-center gap-2.5 rounded-lg px-2 py-1.5 ${r.offInSettings ? 'opacity-60' : ''}`}
                >
                  <ProviderMark provider={r.source.provider} size={14} className={r.included ? '' : 'opacity-40 grayscale'} />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-[13px] font-semibold text-textPrimary">{r.source.channelName}</span>
                    <span className={`block truncate text-[11px] ${r.error ? 'text-warning' : 'text-textMuted'}`}>
                      {r.offInSettings ? `${meta.label} is off in Settings` : r.error ?? meta.label}
                    </span>
                  </span>
                  <Toggle enabled={r.included} onChange={() => !r.offInSettings && onToggle(r.source)} />
                </div>
              );
            })}
            {suggestions.length > 0 && (
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenuOpen(false);
                  onOpenSuggestions();
                }}
                className="mt-1 flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12px] text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
              >
                {suggestions.map((s) => (
                  <ProviderMark key={s.candidate.provider} provider={s.candidate.provider} size={12} />
                ))}
                <span className="truncate">Also streaming on {found.map((p) => PROVIDERS[p].label).join(' and ')}?</span>
                {suggestionsUnseen && <span className="ml-auto h-1.5 w-1.5 flex-shrink-0 rounded-full bg-accent" aria-hidden />}
              </button>
            )}
            <div className="my-1 border-t border-white/[0.06]" />
            <button
              type="button"
              role="menuitem"
              onClick={() => {
                setMenuOpen(false);
                onOpenEditor();
              }}
              className="flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-[12px] font-medium text-textSecondary transition-colors hover:bg-white/[0.06] hover:text-textPrimary"
            >
              <Plus size={11} weight="bold" />
              Link another channel
            </button>
          </div>,
          document.body,
        )}
    </>
  );
}
