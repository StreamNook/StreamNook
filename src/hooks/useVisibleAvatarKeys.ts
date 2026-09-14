// Which stream cards are actually in the viewport right now.
//
// Avatar resolution is cheap for Twitch (100 ids per Helix call) and expensive
// for YouTube category/offline rows (a channel lookup each). With a few hundred
// followed channels, resolving the whole list would cost hundreds of requests for
// cards the user never scrolls to, so resolution is driven by what is on screen.
//
// One shared IntersectionObserver watches every element tagged with
// `data-avatar-key`, rather than a hook per card: the card markup only has to
// carry the attribute, and adding a new grid costs nothing here.

import { useEffect, useRef, useState } from 'react';

/** Start resolving slightly before a card scrolls in, so it is rarely blank by
 *  the time it is looked at. */
const ROOT_MARGIN = '300px';

/**
 * @param deps  Values whose change means the rendered cards changed, so the
 *              observer re-attaches to the new elements.
 */
export function useVisibleAvatarKeys(deps: unknown[]): Set<string> {
    const [visible, setVisible] = useState<Set<string>>(() => new Set());
    // Mirrors `visible` so the observer callback can read the current set without
    // being torn down and rebuilt on every addition. Per instance, never module
    // scope, so a remount starts clean.
    const seenRef = useRef<Set<string>>(new Set());

    useEffect(() => {
        if (typeof IntersectionObserver === 'undefined') return;
        const nodes = Array.from(document.querySelectorAll<HTMLElement>('[data-avatar-key]'));
        if (nodes.length === 0) return;

        // Keys only ever accumulate. A card scrolling back out does not make its
        // avatar wrong, and dropping it would just re-request on the next pass.
        const observer = new IntersectionObserver(
            (entries) => {
                let added = false;
                for (const entry of entries) {
                    if (!entry.isIntersecting) continue;
                    const key = (entry.target as HTMLElement).dataset.avatarKey;
                    if (key && !seenRef.current.has(key)) {
                        seenRef.current.add(key);
                        added = true;
                    }
                }
                // A new Set instance, so consumers memoized on identity re-run.
                if (added) setVisible(new Set(seenRef.current));
            },
            { rootMargin: ROOT_MARGIN },
        );
        nodes.forEach((n) => observer.observe(n));
        return () => observer.disconnect();
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, deps);

    return visible;
}

export default useVisibleAvatarKeys;
