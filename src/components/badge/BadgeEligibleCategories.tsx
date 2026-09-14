import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Gamepad2 } from 'lucide-react';
import { Logger } from '../../utils/logger';
import { categoryLookupNames, readCategoryList } from '../../utils/eligibleCategories';

interface HelixCategory {
  id: string;
  name: string;
  box_art_url?: string;
}

interface Resolved {
  name: string;
  boxArtUrl?: string;
}

// One resolution per list, reused across reopens of the panel.
const cache = new Map<string, Resolved[]>();

/**
 * Reads the "eligible categories" list out of badge copy and turns it into the
 * real categories it names.
 *
 * Every candidate reading goes to Twitch in ONE exact-match request, and the
 * longest reading that comes back wins. That ordering is the whole point: the
 * pieces "Animals, Aquariums, and Zoos" splits into are themselves real
 * categories, so resolving the short ones first would produce three confident
 * wrong answers instead of one right one. Names Twitch doesn't know never
 * render, so a chip always leads somewhere.
 */
async function resolveList(text: string): Promise<Resolved[]> {
  const cached = cache.get(text);
  if (cached) return cached;

  const candidates = categoryLookupNames(text);
  if (candidates.length === 0) {
    cache.set(text, []);
    return [];
  }

  try {
    const found = await invoke<HelixCategory[]>('get_categories_by_name', { names: candidates });
    // Twitch's own casing wins over the copy's ("AQUARIUMS", not "Aquariums").
    const byName = new Map((found ?? []).map((c) => [c.name.toLowerCase(), c]));
    const resolved = readCategoryList(text, (name) => byName.has(name.toLowerCase())).map((name) => {
      const hit = byName.get(name.toLowerCase());
      return { name: hit?.name ?? name, boxArtUrl: hit?.box_art_url };
    });
    cache.set(text, resolved);
    return resolved;
  } catch (err) {
    Logger.warn('[BadgeEligibleCategories] category lookup failed:', err);
    return [];
  }
}

/** Where a badge can be earned, as chips that open the category. */
export const BadgeEligibleCategories = ({
  text,
  onOpen,
}: {
  text: string;
  onOpen: (name: string) => void;
}) => {
  const [categories, setCategories] = useState<Resolved[]>(() => cache.get(text) ?? []);

  useEffect(() => {
    let alive = true;
    // Always goes through resolveList: a cached list resolves immediately, so
    // one path covers both the first open and a reopen.
    void resolveList(text).then((r) => {
      if (alive) setCategories(r);
    });
    return () => {
      alive = false;
    };
  }, [text]);

  if (categories.length === 0) return null;

  return (
    <div>
      <h4 className="text-[12px] font-semibold text-textSecondary uppercase tracking-wide mb-2">
        Eligible categories
      </h4>
      <div className="flex flex-wrap gap-1.5">
        {categories.map((c) => {
          const art = c.boxArtUrl?.replace('{width}', '52').replace('{height}', '72');
          return (
            <button
              key={c.name}
              onClick={() => onOpen(c.name)}
              title={c.name}
              className="group flex items-center gap-2 pl-1 pr-2.5 py-1 rounded-lg bg-white/[0.04] hover:bg-white/[0.07] border border-white/[0.06] transition-colors max-w-full"
            >
              {art ? (
                <img src={art} alt="" className="w-[22px] h-[30px] rounded object-cover shrink-0" loading="lazy" />
              ) : (
                <span className="w-[22px] h-[30px] rounded bg-white/[0.06] flex items-center justify-center shrink-0">
                  <Gamepad2 size={13} className="text-textMuted" />
                </span>
              )}
              <span className="text-[13px] text-textPrimary truncate group-hover:text-accent transition-colors">
                {c.name}
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
};
