import { useState, useRef, useLayoutEffect } from 'react';

interface StreamTileTagsProps {
    tags: string[];
    selectedTags: string[];
    onToggleTag: (tag: string) => void;
}

// Rough width reserved for the "+N" chip when deciding how many tags fit. A hair
// generous so the chip never wraps to a second line on its own.
const CHIP_RESERVE = 46;

// Stream-card tag row. Shows as many tags as fit on a single line, then a "+N"
// chip for the remainder (click to reveal the rest in place). If every tag fits
// on one line, they all show and there's no chip — the count adapts to tile
// width and how long the individual tags are.
export const StreamTileTags = ({ tags, selectedTags, onToggleTag }: StreamTileTagsProps) => {
    const containerRef = useRef<HTMLDivElement>(null);
    const lastWidth = useRef(0);
    // null = "measuring" pass (render every tag so we can read where the line wraps).
    const [visibleCount, setVisibleCount] = useState<number | null>(null);
    const [expanded, setExpanded] = useState(false);

    // Re-measure from scratch whenever the tag set changes. Done during render
    // (adjust-state-on-prop-change) so the measuring pass is the very next
    // render, not one commit later.
    const [measuredTags, setMeasuredTags] = useState(tags);
    if (tags !== measuredTags) {
        setMeasuredTags(tags);
        setVisibleCount(null);
        setExpanded(false);
    }

    // Measure which tags sit on the first line; reserve room for the chip if any wrap.
    useLayoutEffect(() => {
        if (visibleCount !== null || expanded) return;
        const el = containerRef.current;
        if (!el) return;
        const chips = Array.from(el.querySelectorAll<HTMLElement>('[data-tag-chip]'));
        if (chips.length === 0) return;

        const firstTop = chips[0].offsetTop;
        let count = 0;
        for (const chip of chips) {
            if (chip.offsetTop <= firstTop) count++;
            else break;
        }

        if (count < tags.length && count > 0) {
            // Make sure the "+N" chip fits at the end of the first line.
            const last = chips[count - 1];
            const rightEdge = last.offsetLeft + last.offsetWidth;
            if (el.clientWidth - rightEdge < CHIP_RESERVE) count -= 1;
        }
        // Layout measurement: read where the chips wrapped, then commit the
        // count before paint. The synchronous second render is the point of
        // useLayoutEffect here, not an accident.
        // eslint-disable-next-line react-hooks/set-state-in-effect
        setVisibleCount(Math.max(1, count));
    });

    // Re-measure on width changes only (height shifts from collapsing must not loop).
    useLayoutEffect(() => {
        const el = containerRef.current;
        if (!el) return;
        const ro = new ResizeObserver(entries => {
            const w = entries[0].contentRect.width;
            if (Math.abs(w - lastWidth.current) > 1) {
                lastWidth.current = w;
                setVisibleCount(null);
                setExpanded(false);
            }
        });
        ro.observe(el);
        return () => ro.disconnect();
    }, []);

    if (tags.length === 0) return null;

    const showAll = expanded || visibleCount === null;
    const shown = showAll ? tags : tags.slice(0, visibleCount);
    const hidden = tags.length - shown.length;

    return (
        <div ref={containerRef} className="flex flex-wrap gap-1.5 pt-1.5">
            {shown.map(tag => {
                const isActive = selectedTags.some(t => t.toLowerCase() === tag.toLowerCase());
                return (
                    <button
                        key={tag}
                        data-tag-chip
                        onClick={(e) => { e.stopPropagation(); onToggleTag(tag); }}
                        className={`text-[11px] font-semibold px-2 py-[3px] rounded-md truncate max-w-[130px] transition-colors ${
                            isActive
                                ? 'bg-accent/25 text-accent'
                                : 'bg-white/[0.08] text-textSecondary hover:bg-accent/15 hover:text-accent'
                        }`}
                    >
                        {tag}
                    </button>
                );
            })}
            {!showAll && hidden > 0 && (
                <button
                    onClick={(e) => { e.stopPropagation(); setExpanded(true); }}
                    className="text-[11px] font-semibold px-2 py-[3px] rounded-md bg-white/[0.05] text-textSecondary/70 hover:bg-white/[0.12] hover:text-textPrimary transition-colors"
                >
                    +{hidden}
                </button>
            )}
        </div>
    );
};
