// The MultiNook toolbar's layout menu: pick how the streams are arranged by
// looking at it. Each choice is a small picture drawn by the real layout
// engine with the streams actually in the grid, so the preview is the layout
// you get, and it follows the size slider as it moves. The choice is a setting
// Rust keeps (settings.multi_nook_layout); the store's setLayout saves it.
//
// Surfaces are the house glass primitives (`glass-flyout`, `glass-button`,
// `glass-button-active`), which follow the Glassiness setting and go solid at
// 0%.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { LayoutGrid, PanelBottom, PanelRight } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import { usemultiNookStore } from '../../stores/multiNookStore';
import { DEFAULT_MULTI_NOOK_LAYOUT, type MultiNookLayoutMode } from '../../types';
import { nookLayout } from './nookLayout';

const MODES: Array<{ mode: MultiNookLayoutMode; label: string; detail: string; Icon: typeof LayoutGrid }> = [
  { mode: 'grid', label: 'Grid', detail: 'Every stream the same size', Icon: LayoutGrid },
  { mode: 'main_row', label: 'Main + row', detail: 'One large stream, the rest below it', Icon: PanelBottom },
  { mode: 'main_column', label: 'Main + column', detail: 'One large stream, the rest beside it', Icon: PanelRight },
];

const CAPS: Array<{ value: number | null; label: string }> = [
  { value: null, label: 'Same' },
  { value: 720, label: '720p' },
  { value: 480, label: '480p' },
  { value: 360, label: '360p' },
];

const PREVIEW_W = 84;
const PREVIEW_H = 48;

/** The layout as it will look, in miniature. */
function LayoutPreview({ mode, count, share }: { mode: MultiNookLayoutMode; count: number; share: number }) {
  const places = nookLayout(PREVIEW_W, PREVIEW_H, count, mode, share, 3);
  return (
    <span className="relative block" style={{ width: PREVIEW_W, height: PREVIEW_H }} aria-hidden>
      {places.map((p, i) => (
        <span
          key={i}
          className="pointer-events-none absolute rounded-[3px] bg-current"
          style={{ left: p.x, top: p.y, width: p.w, height: p.h, opacity: p.main ? 0.85 : 0.4 }}
        />
      ))}
    </span>
  );
}

export default function MultiNookLayoutPicker() {
  const layout = useAppStore((s) => s.settings.multi_nook_layout) ?? DEFAULT_MULTI_NOOK_LAYOUT;
  const visibleCount = usemultiNookStore((s) => s.slots.filter((x) => !x.isMinimized).length);
  const draftShare = usemultiNookStore((s) => s.draftShare);
  const { setLayout, setDraftShare } = usemultiNookStore.getState();
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; right: number } | null>(null);
  const close = useCallback(() => setIsOpen(false), []);

  const share = draftShare ?? layout.strip_share;
  const mainLayout = layout.mode !== 'grid';
  // Few streams draw a poor picture of a layout; show at least four.
  const previewCount = Math.min(9, Math.max(4, visibleCount));
  const Current = MODES.find((m) => m.mode === layout.mode)?.Icon ?? LayoutGrid;

  // A drag re-lays the grid live through the store's draft; the setting is
  // saved once, when the slider is let go.
  const commitShare = () => {
    const d = usemultiNookStore.getState().draftShare;
    if (d === null) return;
    void setLayout({ strip_share: d }).finally(() => setDraftShare(null));
  };

  // The menu hangs from the button but renders at the document root. Inside
  // the toolbar it would sit in the cluster's own backdrop blur, and a blur
  // nested in another blur only sees the cluster, never the video under it,
  // so the stream came through the menu unfrosted.
  useLayoutEffect(() => {
    if (!isOpen) return;
    const place = () => {
      const r = containerRef.current?.getBoundingClientRect();
      if (r) setMenuPos({ top: r.bottom + 8, right: Math.max(8, window.innerWidth - r.right) });
    };
    place();
    window.addEventListener('resize', place);
    return () => window.removeEventListener('resize', place);
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (containerRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      close();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') close();
    };
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
    };
  }, [isOpen, close]);

  return (
    <div ref={containerRef} className="relative">
      <Tooltip content="Layout" delay={200} side="bottom">
        <button
          onClick={() => setIsOpen((o) => !o)}
          aria-pressed={isOpen}
          aria-label="Layout"
          aria-haspopup="dialog"
          className={`titlebar-icon-btn ${isOpen ? 'is-active !text-accent' : mainLayout ? '!text-accent' : 'hover:!text-accent'}`}
        >
          <Current size={16} />
        </button>
      </Tooltip>

      {isOpen && menuPos && createPortal(
        // glass-flyout, not sn-popover: this opens over live video, where the
        // chat popover's half-clear tint lets moving streams wash out the menu.
        // The flyout keeps a near-opaque base at every Glassiness setting and
        // still frosts what is behind it.
        <div
          ref={menuRef}
          role="dialog"
          aria-label="Layout"
          className="glass-flyout w-[19rem] !p-3"
          style={{ position: 'fixed', top: menuPos.top, right: menuPos.right, zIndex: 9999 }}
        >
          <div className="mb-2.5 flex items-baseline justify-between">
            <p className="text-[13px] font-semibold text-textPrimary">Layout</p>
            <span className="text-[11px] text-textMuted">
              {visibleCount} {visibleCount === 1 ? 'stream' : 'streams'}
            </span>
          </div>

          <div className="grid grid-cols-3 gap-2" role="radiogroup" aria-label="Layout">
            {MODES.map(({ mode, label, detail }) => {
              const on = layout.mode === mode;
              return (
                <button
                  key={mode}
                  role="radio"
                  aria-checked={on}
                  aria-label={`${label}: ${detail}`}
                  title={detail}
                  onClick={() => void setLayout({ mode })}
                  className={`flex flex-col items-center gap-2 rounded-lg px-1.5 pb-2 pt-2.5 text-[11px] font-medium transition-colors ${
                    on ? 'glass-button-active text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'
                  }`}
                >
                  <span className={on ? 'text-accent' : 'text-textMuted'}>
                    <LayoutPreview mode={mode} count={previewCount} share={share} />
                  </span>
                  {label}
                </button>
              );
            })}
          </div>

          {mainLayout && (
            <div className="mt-3 space-y-3.5 border-t border-borderSubtle pt-3">
              <div>
                <div className="flex items-baseline justify-between">
                  <label htmlFor="nook-strip-share" className="text-xs font-medium text-textPrimary">
                    Small streams
                  </label>
                  <span className="text-[11px] tabular-nums text-textMuted">{Math.round(share * 100)}% of the space</span>
                </div>
                <input
                  id="nook-strip-share"
                  type="range"
                  min={15}
                  max={40}
                  step={1}
                  value={Math.round(share * 100)}
                  onChange={(e) => setDraftShare(Number(e.target.value) / 100)}
                  onPointerUp={commitShare}
                  onKeyUp={commitShare}
                  onBlur={commitShare}
                  className="mt-2 w-full cursor-pointer accent-accent"
                />
                <div className="mt-0.5 flex justify-between text-[10px] text-textMuted">
                  <span>Smaller</span>
                  <span>Larger</span>
                </div>
              </div>

              <div>
                <p className="text-xs font-medium text-textPrimary">Small streams play at</p>
                <div className="mt-2 grid grid-cols-4 gap-1.5" role="radiogroup" aria-label="Small streams play at">
                  {CAPS.map(({ value, label }) => {
                    const on = layout.small_quality_cap === value;
                    return (
                      <button
                        key={label}
                        role="radio"
                        aria-checked={on}
                        onClick={() => void setLayout({ small_quality_cap: value })}
                        className={`rounded-md py-1.5 text-[11px] font-semibold transition-colors ${
                          on ? 'glass-button-active text-textPrimary' : 'glass-button text-textSecondary hover:text-textPrimary'
                        }`}
                      >
                        {label}
                      </button>
                    );
                  })}
                </div>
                <p className="mt-2 text-[11px] leading-4 text-textMuted">
                  {layout.small_quality_cap
                    ? 'Uses less bandwidth and CPU. A small stream made main switches back to its own quality.'
                    : 'Every stream keeps its own quality.'}
                </p>
              </div>
            </div>
          )}
        </div>,
        document.body,
      )}
    </div>
  );
}
