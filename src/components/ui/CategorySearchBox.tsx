import { useState, useRef, useEffect, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Search, X, Check, Tag as TagIcon, CornerDownLeft } from 'lucide-react';

export interface CategorySearchBoxProps {
    // Live text in the box. Drives both the stream filter (in the parent) and
    // the tag suggestions below.
    value: string;
    onChange: (value: string) => void;
    // The category's offered tags, shown as defaults when the box is empty.
    tagOptions: string[];
    // Broader pool (e.g. tags carried by live streams) folded into suggestions
    // while typing, so autocomplete reaches beyond the offered tags.
    tagSuggestions?: string[];
    // Currently active tag filters (chips live in the parent).
    selectedTags: string[];
    onToggleTag: (tag: string) => void;
    placeholder?: string;
}

const MAX_VISIBLE = 14;

export const CategorySearchBox = ({
    value,
    onChange,
    tagOptions,
    tagSuggestions = [],
    selectedTags,
    onToggleTag,
    placeholder = '',
}: CategorySearchBoxProps) => {
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    const selectedLower = useMemo(() => new Set(selectedTags.map(t => t.toLowerCase())), [selectedTags]);

    // Deduped pool of every tag we can suggest (offered first, then stream tags).
    const pool = useMemo(() => {
        const seen = new Set<string>();
        const out: string[] = [];
        for (const t of [...tagOptions, ...tagSuggestions]) {
            const key = t.toLowerCase();
            if (key && !seen.has(key)) {
                seen.add(key);
                out.push(t);
            }
        }
        return out;
    }, [tagOptions, tagSuggestions]);

    const q = value.trim().toLowerCase();

    // Empty box -> offered tags as defaults (falling back to the pool). Typing ->
    // narrow to matches, prefix-first so it gets more specific letter by letter.
    const visibleTags = useMemo(() => {
        if (!q) {
            const defaults = tagOptions.length > 0 ? tagOptions : pool;
            return defaults.slice(0, MAX_VISIBLE);
        }
        return pool
            .filter(t => t.toLowerCase().includes(q))
            .sort((a, b) => {
                const ap = a.toLowerCase().startsWith(q) ? 0 : 1;
                const bp = b.toLowerCase().startsWith(q) ? 0 : 1;
                return ap - bp || a.localeCompare(b);
            })
            .slice(0, MAX_VISIBLE);
    }, [q, pool, tagOptions]);

    useEffect(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
                setIsOpen(false);
            }
        };
        if (isOpen) document.addEventListener('mousedown', handleClickOutside);
        return () => document.removeEventListener('mousedown', handleClickOutside);
    }, [isOpen]);

    return (
        <div className="relative w-full max-w-[460px]" ref={containerRef}>
            <div className="relative group">
                <div className="absolute inset-y-0 left-0 pl-3 flex items-center pointer-events-none text-textSecondary group-focus-within:text-accent transition-colors">
                    <Search size={15} />
                </div>
                <input
                    type="text"
                    placeholder={placeholder}
                    value={value}
                    onChange={(e) => { onChange(e.target.value); setIsOpen(true); }}
                    onFocus={() => setIsOpen(true)}
                    onKeyDown={(e) => {
                        if (e.key === 'Enter') {
                            // Apply the best matching tag, or the raw typed text as a
                            // tag if nothing matched (it's still queried server-side).
                            const pick = visibleTags[0] ?? value.trim();
                            if (pick) { onToggleTag(pick); onChange(''); setIsOpen(false); }
                        } else if (e.key === 'Escape') {
                            onChange(''); setIsOpen(false);
                        }
                    }}
                    className={`glass-input pl-9 pr-9 py-2 w-full outline-none text-sm text-textPrimary placeholder-textSecondary/50 font-medium transition-all ${
                        isOpen && visibleTags.length > 0 ? '!rounded-t-lg !rounded-b-none' : '!rounded-lg'
                    }`}
                />
                {value && (
                    <button
                        onClick={() => onChange('')}
                        className="absolute inset-y-0 right-0 pr-3 flex items-center text-textSecondary hover:text-accent transition-colors"
                    >
                        <X size={15} />
                    </button>
                )}
            </div>

            <AnimatePresence>
                {isOpen && (visibleTags.length > 0 || q) && (
                    <motion.div
                        initial={{ opacity: 0, y: -4 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -4 }}
                        transition={{ duration: 0.12, ease: 'easeOut' }}
                        className="absolute left-0 right-0 z-[100] rounded-b-lg bg-glass-ink/95 backdrop-blur-3xl shadow-[0_12px_32px_rgba(0,0,0,0.6)] border-x border-b border-white/10 overflow-hidden"
                    >
                        <div className="px-3 pt-2 pb-1 flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-textSecondary/50">
                            <TagIcon size={11} />
                            {q ? `Tags matching "${value.trim()}"` : 'Suggested tags'}
                        </div>
                        <div className="py-1 max-h-[280px] overflow-y-auto custom-scrollbar">
                            {visibleTags.map((tag) => {
                                const isSelected = selectedLower.has(tag.toLowerCase());
                                return (
                                    <button
                                        key={tag}
                                        onClick={() => { onToggleTag(tag); onChange(''); }}
                                        className={`w-full text-left px-3 py-1.5 text-sm flex items-center gap-2 transition-colors outline-none ${
                                            isSelected ? 'text-accent font-bold bg-white/5' : 'text-textPrimary hover:bg-white/10 hover:text-white'
                                        }`}
                                    >
                                        <span className={`w-3.5 shrink-0 ${isSelected ? 'opacity-100' : 'opacity-0'}`}>
                                            <Check size={14} className="text-accent" />
                                        </span>
                                        <span className="truncate">{tag}</span>
                                    </button>
                                );
                            })}
                        </div>
                        {/* Always offer to apply the raw text as a tag — our suggestion
                            list only knows tags from loaded streams, but the server can
                            filter by any tag name (e.g. ARAM). */}
                        {q && (
                            <button
                                onClick={() => { onToggleTag(value.trim()); onChange(''); setIsOpen(false); }}
                                className="w-full text-left px-3 py-2 text-sm flex items-center gap-2 border-t border-white/5 text-textSecondary hover:bg-white/10 hover:text-accent transition-colors outline-none"
                            >
                                <CornerDownLeft size={13} className="shrink-0" />
                                <span className="truncate">Filter by tag <span className="font-bold text-textPrimary">"{value.trim()}"</span></span>
                            </button>
                        )}
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
};
