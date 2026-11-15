// Source pins for the category page's loaders. The render tests next to this
// file prove what happens; these prove it keeps happening for every loader,
// including the ones no render test reaches (the load-mores, the by-name loader).
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const read = (p: string) => readFileSync(resolve(__dirname, p), 'utf8').replace(/\r\n/g, '\n');
const home = read('Home.tsx');
const squash = (s: string) => s.replace(/\s+/g, ' ');

// Text from `from` up to and including `to`, or the test fails by name.
const between = (src: string, from: string, to: string) => {
  const start = src.indexOf(from);
  expect(start, `missing: ${from}`).toBeGreaterThanOrEqual(0);
  const end = src.indexOf(to, start);
  expect(end, `missing: ${to}`).toBeGreaterThan(start);
  return src.slice(start, end + to.length);
};

// The design's invoke regex: every category call in Home, typed calls included.
const CATEGORY_INVOKE = /invoke(<[^>]*>)?\('get_(streams_by_game|streams_by_game_name|clips_by_game|videos_by_game|category_info)'/g;

const FLAG_WRITE =
  /\bset(CategoryStreams|CategoryStreamsCursor|HasMoreCategoryStreams|CategoryClips|CategoryClipsCursor|HasMoreCategoryClips|CategoryVideos|CategoryVideosCursor|HasMoreCategoryVideos|CategoryDetails|IsLoading\w+)\(/;

describe('every category loader in Home', () => {
  const matches = [...home.matchAll(CATEGORY_INVOKE)];

  it('is exactly these twelve calls, in file order', () => {
    // A thirteenth call is a loader nobody guarded; a missing one means a loader moved.
    expect(matches.map((m) => m[2])).toEqual([
      'streams_by_game',
      'category_info',
      'streams_by_game_name',
      'category_info',
      'streams_by_game',
      'category_info',
      'streams_by_game',
      'streams_by_game_name',
      'clips_by_game',
      'clips_by_game',
      'videos_by_game',
      'videos_by_game',
    ]);
  });

  it('checks isCurrentLoad( after its invoke and before any data or flag write', () => {
    expect(matches.length).toBe(12);
    matches.forEach((m, i) => {
      const from = m.index!;
      const to = i + 1 < matches.length ? matches[i + 1].index! : from + 3500;
      const segment = home.slice(from, to);
      const guard = segment.indexOf('isCurrentLoad(');
      expect(guard, `no isCurrentLoad( after invoke #${i + 1} (${m[2]})`).toBeGreaterThan(0);
      const write = segment.search(FLAG_WRITE);
      if (write >= 0) expect(guard, `write before the guard after invoke #${i + 1}`).toBeLessThan(write);
    });
  });

  it('clears a loading flag only for the load that is still current', () => {
    const lines = home.split('\n').filter((l) => /\bsetIsLoading(CategoryStreams|CategoryDetails|Clips|Videos|MoreCategoryStreams|MoreClips|MoreVideos)\(false\)/.test(l));
    const reset = between(home, 'const resetCategoryLoads = useCallback(', '}, [setCategoryActiveTab]);');
    const loose = lines.filter((l) => !l.includes('isCurrentLoad('));
    // The only unguarded clears are the reset's own: it is what makes older loads stale.
    expect(lines.length).toBeGreaterThanOrEqual(14);
    for (const l of loose) expect(reset).toContain(l.trim());
    expect(loose.length).toBe(7);
  });

  it('reads the ref only to decide a pass or to judge a landing answer, never before its own invoke', () => {
    const reads = [...home.matchAll(/loadedCategoryKeyRef\.current(?!\s*=\s*\{)/g)].map(
      (m) =>
        home.slice(m.index!).match(/^loadedCategoryKeyRef\.current(\?\.key === key|;)/)?.[0] ??
        `OTHER: ${home.slice(m.index!, m.index! + 60)}`,
    );
    // The effect's `prev`, and isCurrentLoad's comparison: nothing else.
    expect(reads.sort()).toEqual(['loadedCategoryKeyRef.current;', 'loadedCategoryKeyRef.current?.key === key'].sort());
    // Each load writes the ref itself in exactly three places, each before it fetches.
    expect(home.match(/loadedCategoryKeyRef\.current = \{ key, name: [\w.]+ \};/g)).toEqual([
      'loadedCategoryKeyRef.current = { key, name: category.name };',
      'loadedCategoryKeyRef.current = { key, name: gameName };',
      'loadedCategoryKeyRef.current = { key, name: selectedCategory.name };',
    ]);
  });

  it('judges a landing answer by the key it fetched for and its own kind\'s token', () => {
    expect(squash(home)).toContain(
      'loadedCategoryKeyRef.current?.key === key && loadTokensRef.current[kind] === token',
    );
    for (const kind of ['streams', 'details', 'clips', 'videos', 'moreStreams', 'moreClips', 'moreVideos']) {
      expect(home).toContain(`startCategoryLoad('${kind}')`);
    }
    // A key captured from the category being fetched, never read from the ref.
    expect(home).not.toMatch(/const key = loadedCategoryKeyRef/);
  });

  it('is reset by the shared reset: every flag false and every token bumped, from the click, the by-name load and the effect', () => {
    const reset = between(home, 'const resetCategoryLoads = useCallback(', '}, [setCategoryActiveTab]);');
    for (const flag of [
      'setIsLoadingCategoryStreams(false)',
      'setIsLoadingCategoryDetails(false)',
      'setIsLoadingClips(false)',
      'setIsLoadingVideos(false)',
      'setIsLoadingMoreCategoryStreams(false)',
      'setIsLoadingMoreClips(false)',
      'setIsLoadingMoreVideos(false)',
    ]) {
      expect(reset).toContain(flag);
    }
    expect(reset).toContain('loadTokensRef.current[kind] += 1');
    const click = between(home, 'const handleCategoryClick = async', '\n    };');
    const byName = between(home, 'const loadCategoryStreamsByName = async', '\n    };');
    for (const body of [click, byName]) {
      expect(body.indexOf('loadedCategoryKeyRef.current = {')).toBeGreaterThan(0);
      expect(body.indexOf('resetCategoryLoads();')).toBeGreaterThan(body.indexOf('loadedCategoryKeyRef.current = {'));
      expect(body.indexOf('invo' + 'ke(')).toBeGreaterThan(body.indexOf('resetCategoryLoads();'));
    }
  });
});

describe('the one reload effect', () => {
  const effect = between(
    home,
    '// The one place a changed category selection reloads the page',
    '}, [activeTab, selectedCategory?.id, selectedCategory?.name]);',
  );

  it('reads prev, decides with it, writes the ref, and only then fetches', () => {
    const prev = effect.indexOf('const prev = loadedCategoryKeyRef.current;');
    const reload = effect.indexOf('categoryNeedsReload(prev?.key ?? null, selectedCategory)');
    const fill = effect.indexOf('categoryFillOnly(prev, selectedCategory)');
    const write = effect.indexOf('loadedCategoryKeyRef.current = { key, name: selectedCategory.name };');
    // The word is split because invokeParity.test.ts scans test files for invoke calls.
    const firstFetch = effect.indexOf('invo' + 'ke(');
    expect(prev).toBeGreaterThan(0);
    expect(reload).toBeGreaterThan(prev);
    expect(fill).toBeGreaterThan(prev);
    expect(write).toBeGreaterThan(Math.max(reload, fill));
    // A ref written at the end of the pass, or after a fetch started, makes the first answer stale.
    expect(firstFetch).toBeGreaterThan(write);
    expect(effect).toContain('categoryShownRef.current = showing;');
  });

  it('skips the streams fetch in a fill-only pass and the details wait for a name', () => {
    expect(squash(effect)).toContain('if (!fillOnly && (reload ||');
    expect(squash(effect)).toContain('if (!selectedCategory.name) {');
  });
});

describe('the rest of the category page', () => {
  it('keys the clips and videos effect on the id, not the object', () => {
    expect(home).toContain('}, [activeTab, categoryActiveTab, selectedCategory?.id]);');
    expect(home).not.toContain('}, [activeTab, categoryActiveTab, selectedCategory]);');
  });

  it('keys the filter box on the selection, not the name', () => {
    expect(home).toContain('const categoryScreenKey = selectedCategory ? categorySelectionKey(selectedCategory) : \'\';');
    expect(home).toContain('}, [categoryScreenKey]);');
    expect(home).not.toContain('}, [selectedCategoryKey]);');
  });

  it('words every empty note through categoryNote, with no raw name in a sentence', () => {
    expect(home).toContain("categoryNote('live', selectedCategory?.name ?? '')");
    expect(home).toContain("categoryNote('clips', selectedCategory?.name ?? '')");
    expect(home).toContain("categoryNote('videos', selectedCategory?.name ?? '')");
    expect(home).not.toMatch(/No one is streaming \$\{/);
    expect(home).not.toMatch(/No clips have been generated for \{/);
    expect(home).not.toMatch(/No videos exist for \{/);
  });

  it('makes its automatic tab choice through autoSelectHomeTab', () => {
    const effect = between(home, '// Auto-select the appropriate tab based on auth status', '}, [isAuthenticated, followedStreams.length]);');
    expect(effect).toContain("useAppStore.getState().autoSelectHomeTab('following')");
    expect(effect).toContain("useAppStore.getState().autoSelectHomeTab('recommended')");
    expect(effect).not.toContain('setActiveTab(');
  });

  it('leaves the control byte in the selectedTagsKey line alone', () => {
    // An edit that rewrote this line would have turned the byte into something else.
    const line = home.split('\n').find((l) => l.includes('const selectedTagsKey'))!;
    expect(line).toContain('.join(\'\u0001\');');
    expect(home.split('\u0001').length).toBe(2);
  });
});

describe('the Share button', () => {
  const handler = between(home, 'const handleShareCategory = async () => {', '\n    };');

  it('has its failure caught in one place, never here', () => {
    expect(handler).toContain('copyCategoryShareLink(');
    expect(handler).not.toContain('.catch(');
    expect(handler).not.toContain('try {');
    // "Link copied!" only after the helper said true.
    expect(handler.indexOf('if (!copied')).toBeGreaterThan(0);
    expect(handler.indexOf('setCategoryShareCopied(true)')).toBeGreaterThan(handler.indexOf('if (!copied'));
  });

  it('passes a bound writer, never the bare clipboard method', () => {
    expect(handler).toContain('(text) => navigator.clipboard.writeText(text)');
    // An unbound writeText throws "Illegal invocation", which the one catch would turn into a silent false.
    expect(home).not.toMatch(/navigator\.clipboard\.writeText(?!\()/);
  });

  it('uses the app Tooltip and an aria-label, with no title attribute', () => {
    const at = home.indexOf('aria-label="Share category"');
    expect(at).toBeGreaterThan(0);
    expect(home.indexOf('aria-label="Share category"', at + 1)).toBe(-1);
    const button = home.slice(home.lastIndexOf('<button', at), home.indexOf('</button>', at));
    expect(button).not.toContain('title=');
    expect(home.slice(home.lastIndexOf('<Tooltip', at), at)).toContain('Tooltip content=');
  });
});

describe('the other places that name a category back', () => {
  it('say "Back to category" for a copy with no name', () => {
    expect(read('NavFlipper.tsx')).toContain("reenter.name ? `Back to ${reenter.name}` : 'Back to category'");
    const player = read('VideoPlayer.tsx');
    expect(player).toContain(
      "content={streamOriginCategory ? (streamOriginCategory.name ? `Back to ${streamOriginCategory.name}` : 'Back to category') : 'Back'}",
    );
    // The stream branch already said it; nothing else builds this label from the bare name.
    expect(player.match(/`Back to \$\{streamOriginCategory\.name\}`/g)).toHaveLength(1);
  });
});
