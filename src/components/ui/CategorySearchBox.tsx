import { useState, useRef, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { Search, X, Check, Tag as TagIcon, CornerDownLeft } from 'lucide-react';
import { useCategoryTagSuggestions } from '../../hooks/useCategoryTagSuggestions';
import { hasTag } from '../../utils/tagFilter';

export interface CategorySearchBoxProps {
    // Text in the box: Rust's tag matches for it drop down below.
    value: string;
    onChange: (value: string) => void;
    // The category whose tags are offered. Rust pools them (the category's own
    // tags and the tags its loaded streams carry) and ranks the matches.
    gameName: string | null;
    // Changes whenever the category's loaded streams change, so the list
    // picks up the tags they brought.
    revision: unknown;
    // Currently active tag filters (chips live in the parent).
    selectedTags: readonly string[];
    onToggleTag: (tag: string) => void;
    placeholder?: string;
}

const MAX_VISIBLE = 14;

export const CategorySearchBox = ({
    value,
    onChange,
    gameName,
    revision,
    selectedTags,
    onToggleTag,
    placeholder = '',
}: CategorySearchBoxProps) => {
    const [isOpen, setIsOpen] = useState(false);
    const containerRef = useRef<HTMLDivElement>(null);

    // Empty box: picks first, then the category's tags, then common tags of
    // its streams. Typing: matches, prefix first. Asked only while open.
    const answer = useCategoryTagSuggestions({
        gameName,
        query: value,
        selected: selectedTags,
        limit: MAX_VISIBLE,
        enabled: isOpen,
        revision,
    });
    const visibleTags = answer?.labels ?? [];
    // Set when no listed tag is the typed text: Twitch filters by any tag name.
    const typedTag = answer?.offer_typed ?? null;
    const q = value.trim();

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
                            // Only typed text picks: the best match Rust gave for
                            // exactly this text, else the text itself as a tag (it
                            // is still queried server-side).
                            if (!q) return;
                            const pick = visibleTags[0] ?? q;
                            onToggleTag(pick); onChange(''); setIsOpen(false);
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
                {isOpen && (visibleTags.length > 0 || typedTag) && (
                    <motion.div
                        initial={{ opacity: 0, y: -4 }}
                        animate={{ opacity: 1, y: 0 }}
                        exit={{ opacity: 0, y: -4 }}
                        transition={{ duration: 0.12, ease: 'easeOut' }}
                        className="absolute left-0 right-0 z-[100] rounded-b-lg bg-glass-ink/95 backdrop-blur-3xl shadow-[0_12px_32px_rgba(0,0,0,0.6)] border-x border-b border-white/10 overflow-hidden"
                    >
                        <div className="px-3 pt-2 pb-1 flex items-center gap-1.5 text-[10px] font-bold uppercase tracking-wider text-textSecondary/50">
                            <TagIcon size={11} />
                            {q ? `Tags matching "${q}"` : 'Suggested tags'}
                        </div>
                        <div className="py-1 max-h-[280px] overflow-y-auto custom-scrollbar">
                            {visibleTags.map((tag) => {
                                const isSelected = hasTag(selectedTags, tag);
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
                        {/* The typed text as a tag, whenever no listed tag is it:
                            the server can filter by any tag name (e.g. ARAM). */}
                        {typedTag && (
                            <button
                                onClick={() => { onToggleTag(typedTag); onChange(''); setIsOpen(false); }}
                                className="w-full text-left px-3 py-2 text-sm flex items-center gap-2 border-t border-white/5 text-textSecondary hover:bg-white/10 hover:text-accent transition-colors outline-none"
                            >
                                <CornerDownLeft size={13} className="shrink-0" />
                                <span className="truncate">Filter by tag <span className="font-bold text-textPrimary">"{typedTag}"</span></span>
                            </button>
                        )}
                    </motion.div>
                )}
            </AnimatePresence>
        </div>
    );
};
