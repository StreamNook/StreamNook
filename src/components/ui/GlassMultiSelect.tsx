import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { motion, AnimatePresence } from 'framer-motion';
import { ChevronDown, Check } from 'lucide-react';
import type { DropdownOption } from './GlassSelect';

export interface GlassMultiSelectProps {
    values: string[];
    onChange: (values: string[]) => void;
    options: DropdownOption[];
    /** Trigger label when nothing is selected; also the clear-all row. */
    emptyLabel: string;
    className?: string;
}

/** Multi-select sibling of GlassSelect: same visual language, but option rows
 *  toggle membership without closing. The menu is portalled to <body> with
 *  fixed coordinates (the ui/Dropdown pattern) so overflow-hidden or scrolling
 *  ancestors, like the settings dialog, can never clip it. */
export const GlassMultiSelect = ({
    values,
    onChange,
    options,
    emptyLabel,
    className = '',
}: GlassMultiSelectProps) => {
    const [isOpen, setIsOpen] = useState(false);
    const triggerRef = useRef<HTMLButtonElement>(null);
    const menuRef = useRef<HTMLDivElement>(null);
    const [menuStyle, setMenuStyle] = useState<React.CSSProperties>({});

    const reposition = () => {
        const el = triggerRef.current;
        if (!el) return;
        const r = el.getBoundingClientRect();
        const width = Math.max(r.width, 190);
        const left = Math.max(8, Math.min(r.right - width, window.innerWidth - width - 8));
        setMenuStyle({
            position: 'fixed',
            top: Math.round(r.bottom + 6),
            left: Math.round(left),
            width: Math.round(width),
            zIndex: 9999,
        });
    };

    useLayoutEffect(() => {
        if (isOpen) reposition();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isOpen]);

    useEffect(() => {
        if (!isOpen) return;
        const onDown = (e: MouseEvent) => {
            const t = e.target as Node;
            if (triggerRef.current?.contains(t) || menuRef.current?.contains(t)) return;
            setIsOpen(false);
        };
        const onKey = (e: KeyboardEvent) => {
            if (e.key === 'Escape') setIsOpen(false);
        };
        // Keep the portalled menu glued to the trigger while ancestors scroll or
        // the window resizes; scrolling inside the option list itself is ignored.
        const onResize = () => reposition();
        const onScroll = (e: Event) => {
            if (menuRef.current?.contains(e.target as Node)) return;
            reposition();
        };
        document.addEventListener('mousedown', onDown);
        document.addEventListener('keydown', onKey);
        window.addEventListener('resize', onResize);
        window.addEventListener('scroll', onScroll, true);
        return () => {
            document.removeEventListener('mousedown', onDown);
            document.removeEventListener('keydown', onKey);
            window.removeEventListener('resize', onResize);
            window.removeEventListener('scroll', onScroll, true);
        };
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [isOpen]);

    const selected = options.filter(o => values.includes(o.value));
    const triggerLabel =
        selected.length === 0
            ? emptyLabel
            : selected.length === 1
                ? selected[0].label
                : `${selected[0].label} +${selected.length - 1}`;

    const toggle = (value: string) => {
        onChange(values.includes(value) ? values.filter(v => v !== value) : [...values, value]);
    };

    return (
        <>
            <button
                ref={triggerRef}
                type="button"
                aria-haspopup="listbox"
                aria-expanded={isOpen}
                onClick={() => setIsOpen(o => !o)}
                className={`glass-input !rounded-lg outline-none text-sm text-textPrimary font-medium transition-all px-3 py-1.5 flex items-center justify-between gap-3 min-w-[130px] cursor-pointer ${className} ${isOpen ? 'ring-1 ring-accent bg-white/10' : ''}`}
            >
                <span className="truncate">{triggerLabel}</span>
                <ChevronDown
                    size={14}
                    className={`text-textSecondary shrink-0 transition-transform duration-300 ${isOpen ? '-rotate-180 text-accent' : ''}`}
                />
            </button>

            {createPortal(
                <AnimatePresence>
                    {isOpen && (
                        <motion.div
                            ref={menuRef}
                            role="listbox"
                            aria-multiselectable
                            style={menuStyle}
                            initial={{ opacity: 0, y: -4, scale: 0.97 }}
                            animate={{ opacity: 1, y: 0, scale: 1 }}
                            exit={{ opacity: 0, y: -4, scale: 0.97 }}
                            transition={{ duration: 0.14, ease: 'easeOut' }}
                            className="rounded-lg bg-[#09090b]/90 backdrop-blur-3xl shadow-[0_8px_32px_rgba(0,0,0,0.6)] border border-white/10 overflow-hidden"
                        >
                            <div className="py-1 flex flex-col w-full max-h-72 overflow-y-auto scrollbar-thin">
                                <button
                                    role="option"
                                    aria-selected={values.length === 0}
                                    onClick={() => {
                                        onChange([]);
                                        setIsOpen(false);
                                    }}
                                    className={`w-full text-left px-3 py-2 text-sm flex items-center justify-between transition-colors outline-none
                                        ${values.length === 0
                                            ? 'text-accent font-bold bg-white/5'
                                            : 'text-textPrimary hover:bg-white/10 hover:text-white focus:bg-white/10'
                                        }
                                    `}
                                >
                                    <span className="truncate">{emptyLabel}</span>
                                    {values.length === 0 && <Check size={14} className="text-accent shrink-0 ml-2" />}
                                </button>
                                {options.map((option) => {
                                    const isSelected = values.includes(option.value);
                                    return (
                                        <button
                                            key={option.value}
                                            role="option"
                                            aria-selected={isSelected}
                                            onClick={() => toggle(option.value)}
                                            className={`w-full text-left px-3 py-2 text-sm flex items-center justify-between transition-colors outline-none
                                                ${isSelected
                                                    ? 'text-accent font-bold bg-white/5'
                                                    : 'text-textPrimary hover:bg-white/10 hover:text-white focus:bg-white/10'
                                                }
                                            `}
                                        >
                                            <span className="truncate">{option.label}</span>
                                            {isSelected && <Check size={14} className="text-accent shrink-0 ml-2" />}
                                        </button>
                                    );
                                })}
                            </div>
                        </motion.div>
                    )}
                </AnimatePresence>,
                document.body,
            )}
        </>
    );
};
