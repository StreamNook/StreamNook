// The chat header's "more" menu: the occasional, mode-changing actions (pin
// this chat, pop it out, float it over other apps) in words, behind one
// button, so the header's always-visible buttons stay the ones used minute to
// minute. Same glass panel, portal and dismissal as the app's Dropdown.
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { AnimatePresence, motion } from 'framer-motion';
import { DotsThree } from 'phosphor-react';
import { Tooltip } from '../ui/Tooltip';

export interface ChatHeaderMenuItem {
  key: string;
  icon: ReactNode;
  label: string;
  /** One short line under the label: what the action does. */
  detail?: string;
  /** Shown as on (the chat is pinned, say). */
  active?: boolean;
  onSelect: () => void;
}

const MENU_WIDTH = 236;

export default function ChatHeaderMenu({ items }: { items: ChatHeaderMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [style, setStyle] = useState<React.CSSProperties>({});

  const place = () => {
    const r = triggerRef.current?.getBoundingClientRect();
    if (!r) return;
    const left = Math.max(8, Math.min(r.right - MENU_WIDTH, window.innerWidth - MENU_WIDTH - 8));
    setStyle({ position: 'fixed', top: Math.round(r.bottom + 6), left: Math.round(left), width: MENU_WIDTH, zIndex: 9999 });
  };

  useLayoutEffect(() => {
    if (open) place();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (triggerRef.current?.contains(t) || menuRef.current?.contains(t)) return;
      setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setOpen(false);
    };
    const onMove = () => place();
    document.addEventListener('mousedown', onDown);
    document.addEventListener('keydown', onKey);
    window.addEventListener('resize', onMove);
    window.addEventListener('scroll', onMove, true);
    return () => {
      document.removeEventListener('mousedown', onDown);
      document.removeEventListener('keydown', onKey);
      window.removeEventListener('resize', onMove);
      window.removeEventListener('scroll', onMove, true);
    };
  }, [open]);

  if (items.length === 0) return null;

  return (
    <>
      <Tooltip content="More" side="bottom">
        <button
          ref={triggerRef}
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            setOpen((o) => !o);
          }}
          className={`chat-header-btn ${open ? 'is-active' : ''}`}
          aria-label="More chat actions"
          aria-haspopup="menu"
          aria-expanded={open}
        >
          <DotsThree size={16} weight="bold" />
        </button>
      </Tooltip>
      {createPortal(
        <AnimatePresence>
          {open && (
            <motion.div
              ref={menuRef}
              role="menu"
              style={style}
              initial={{ opacity: 0, y: -4, scale: 0.97 }}
              animate={{ opacity: 1, y: 0, scale: 1 }}
              exit={{ opacity: 0, y: -4, scale: 0.97 }}
              transition={{ duration: 0.14, ease: 'easeOut' }}
              className="glass-panel rounded-lg border border-borderLight p-1 shadow-xl"
            >
              {items.map((item) => (
                <button
                  key={item.key}
                  type="button"
                  role="menuitem"
                  onClick={(e) => {
                    e.stopPropagation();
                    setOpen(false);
                    item.onSelect();
                  }}
                  className="flex w-full items-start gap-2.5 rounded-md px-2.5 py-1.5 text-left transition-colors hover:bg-glass-hover"
                >
                  <span className={`mt-0.5 flex shrink-0 items-center ${item.active ? 'text-accent' : 'text-textSecondary'}`}>
                    {item.icon}
                  </span>
                  <span className="flex min-w-0 flex-col">
                    <span className="truncate text-sm text-textPrimary">{item.label}</span>
                    {item.detail && <span className="text-[11px] leading-4 text-textMuted">{item.detail}</span>}
                  </span>
                </button>
              ))}
            </motion.div>
          )}
        </AnimatePresence>,
        document.body,
      )}
    </>
  );
}
