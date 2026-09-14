// CommandPaletteSettings: settings tab that documents every Command Palette
// feature AND lets the user manage their snippet library (favorites, aliases,
// custom additions).
//
// The wiki is intentionally inline rather than linked to external docs so it
// stays in lockstep with the actual implementation. When a new mode prefix
// or section ships, the table here gets updated in the same PR.

import { useMemo, useState } from 'react';
import { Star, Pencil, Trash2, Plus, Save, X as XIcon, Copy } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { Dropdown } from '../ui/Dropdown';
import { useSnippetStore } from '../../stores/snippetStore';
import { getBuiltInSnippets, type Snippet } from '../../utils/commandPaletteCopypastas';
import { copyToClipboard } from '../../utils/commandPaletteSources';
import { SettingsSection } from './_primitives';

const SNIPPET_CATEGORIES: Snippet['category'][] = [
  'Classic',
  'Hype',
  'Reaction',
  'F / RIP',
  'Forsen Lore',
  'Long Classic',
  'Meme',
  'Chat Commands',
];

export default function CommandPaletteSettings() {
  return (
    <div className="space-y-8">
      <Hero />
      <KeyboardShortcuts />
      <SectionsWalkthrough />
      <SnippetManager />
    </div>
  );
}

// ---------- Hero ------------------------------------------------------------

function Hero() {
  const openCommandPalette = useAppStore((s) => s.openCommandPalette);
  const closeSettings = useAppStore((s) => s.closeSettings);
  return (
    <div className="rounded-xl border border-white/5 bg-gradient-to-br from-white/[0.04] to-white/[0.01] p-5">
      <p className="text-sm text-textSecondary leading-relaxed">
        Press <Kbd>Ctrl</Kbd> + <Kbd>K</Kbd> anywhere in StreamNook (main app or any MultiChat popout) to bring
        up a quick-search panel. Jump to settings, find streamers, copy a copypasta, and 30+ other actions
        from one keystroke.
      </p>
      <button
        type="button"
        onClick={() => {
          closeSettings();
          setTimeout(() => openCommandPalette(), 60);
        }}
        className="mt-4 inline-flex items-center gap-2 rounded-lg bg-accent px-4 py-2 text-sm font-semibold text-background transition-colors hover:bg-accent-hover"
      >
        Try it now <Kbd className="bg-background/30 border-background/50 text-background">Ctrl+K</Kbd>
      </button>
    </div>
  );
}

// ---------- Keyboard shortcuts ---------------------------------------------

function KeyboardShortcuts() {
  return (
    <SettingsSection id="settings-section-keyboard" label="Keyboard Shortcuts">
      <ShortcutRow chord={['Ctrl', 'K']} desc="Open or close the command palette (also Cmd+K on macOS)" />
      <ShortcutRow chord={['↑']} desc="Move selection up" />
      <ShortcutRow chord={['↓']} desc="Move selection down" />
      <ShortcutRow chord={['Enter']} desc="Execute the selected row" />
      <ShortcutRow chord={['Esc']} desc="Close the palette" />
      <ShortcutRow chord={['Home']} desc="Jump to the first result" />
      <ShortcutRow chord={['End']} desc="Jump to the last result" />
    </SettingsSection>
  );
}

function ShortcutRow({ chord, desc }: { chord: string[]; desc: string }) {
  return (
    <div className="settings-row -mx-4 px-4 py-3 flex items-center justify-between gap-4">
      <div className="text-sm text-textSecondary">{desc}</div>
      <div className="flex items-center gap-1">
        {chord.map((k, i) => (
          <span key={i} className="inline-flex items-center">
            {i > 0 && <span className="mx-1 text-textMuted">+</span>}
            <Kbd>{k}</Kbd>
          </span>
        ))}
      </div>
    </div>
  );
}

// ---------- Sections walkthrough -------------------------------------------

function SectionsWalkthrough() {
  return (
    <SettingsSection label="What lives in the palette" bare>
      <div className="grid gap-2">
        <SectionRow name="Quick Actions" desc="Verbs that always apply: open Drops/Badges/Whispers, surprise-me, refresh follows, sleep timers, feedback links." />
        <SectionRow name="Current Stream" desc="Only useful while watching a stream: pop chat out, theatre mode, restart/stop, follow/unfollow, view drops for this game, browse other streams of this game." />
        <SectionRow name="Share" desc="Copy stream URL, markdown link, share text, embed iframe; open VODs / Clips / Schedule / About on twitch.tv." />
        <SectionRow name="Settings" desc="Every settings tab and section is searchable. Type 'ad block' to land on the ad blocking panel under Integrations." />
        <SectionRow name="Categories" desc="Type a game name. Each match expands to Browse {Game} and View drops for {Game}." />
        <SectionRow name="Followed Channels" desc="Live snapshot of your following list. Selecting starts watching." />
        <SectionRow name="Recent Chatters" desc="People who've spoken in the current chat. Selecting opens whispers with them." />
        <SectionRow name="Streamers" desc="Live and offline Twitch channels matching what you typed. Results appear once you have typed 2 or more characters." />
        <SectionRow name="Snippets" desc="Copypastas + Twitch slash-command snippets. Selecting copies the body to your clipboard. Star to favorite; set an alias for instant matching." />
        <SectionRow name="Recent" desc="Your last 6 picks, surfaced when the palette is empty." />
      </div>
    </SettingsSection>
  );
}

function SectionRow({ name, desc }: { name: string; desc: string }) {
  return (
    <div className="rounded-lg border border-white/5 bg-white/[0.02] px-4 py-3">
      <div className="text-sm font-semibold text-textPrimary">{name}</div>
      <div className="text-[12px] text-textSecondary mt-0.5 leading-relaxed">{desc}</div>
    </div>
  );
}

// ---------- Snippet manager -------------------------------------------------

function SnippetManager() {
  const customSnippets = useSnippetStore((s) => s.customSnippets);
  const favoriteIds = useSnippetStore((s) => s.favoriteIds);
  const aliases = useSnippetStore((s) => s.aliases);
  const toggleFavorite = useSnippetStore((s) => s.toggleFavorite);
  const setAlias = useSnippetStore((s) => s.setAlias);
  const removeCustomSnippet = useSnippetStore((s) => s.removeCustomSnippet);

  const [query, setQuery] = useState('');
  const [categoryFilter, setCategoryFilter] = useState<Snippet['category'] | 'All' | 'Favorites'>('All');
  const [editingId, setEditingId] = useState<string | null>(null);
  const [showAddForm, setShowAddForm] = useState(false);

  const allSnippets = useMemo<Snippet[]>(
    () => [...getBuiltInSnippets(), ...customSnippets],
    [customSnippets],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return allSnippets.filter((s) => {
      if (categoryFilter === 'Favorites') {
        if (!favoriteIds.has(s.id)) return false;
      } else if (categoryFilter !== 'All') {
        if (s.category !== categoryFilter) return false;
      }
      if (!q) return true;
      return (
        s.title.toLowerCase().includes(q) ||
        s.content.toLowerCase().includes(q) ||
        s.category.toLowerCase().includes(q) ||
        (s.keywords ?? '').toLowerCase().includes(q)
      );
    });
  }, [allSnippets, query, categoryFilter, favoriteIds]);

  return (
    <section id="settings-section-snippets">
      <div className="flex items-center justify-between gap-3 mb-3">
        <div>
          <h3 className="text-[12px] font-semibold uppercase tracking-[0.14em] text-textPrimary">Snippet Manager</h3>
          <p className="text-xs text-textSecondary mt-0.5">
            Star the snippets you use most, bind aliases for instant matching, and add your own.
          </p>
        </div>
        <button
          type="button"
          onClick={() => setShowAddForm((v) => !v)}
          className="inline-flex items-center gap-2 rounded-lg bg-accent/15 hover:bg-accent/25 border border-transparent px-3 py-2 text-sm font-semibold text-accent transition-colors"
        >
          <Plus size={14} /> {showAddForm ? 'Hide form' : 'Add custom'}
        </button>
      </div>

      {showAddForm && <AddCustomForm onDone={() => setShowAddForm(false)} />}

      <div className="flex flex-wrap items-center gap-2 mb-3">
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search snippets…"
          aria-label="Search snippets"
          className="flex-1 min-w-[200px] rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm text-textPrimary placeholder:text-textMuted focus:border-accent/60 focus:outline-none"
        />
        <CategoryPill label="All" active={categoryFilter === 'All'} onClick={() => setCategoryFilter('All')} />
        <CategoryPill label="★ Favorites" active={categoryFilter === 'Favorites'} onClick={() => setCategoryFilter('Favorites')} />
        {SNIPPET_CATEGORIES.map((c) => (
          <CategoryPill key={c} label={c} active={categoryFilter === c} onClick={() => setCategoryFilter(c)} />
        ))}
      </div>

      <div className="text-[11px] text-textMuted mb-2">
        {filtered.length} of {allSnippets.length} · {favoriteIds.size} favorited · {aliases.size} aliased · {customSnippets.length} custom
      </div>

      <div className="rounded-lg border border-white/5 hairline-y max-h-[420px] overflow-y-auto scrollbar-thin">
        {filtered.length === 0 ? (
          <div className="px-4 py-10 text-center text-sm text-textMuted">No snippets match your filters.</div>
        ) : (
          filtered.map((s) => (
            <SnippetRow
              key={s.id}
              snippet={s}
              isFavorite={favoriteIds.has(s.id)}
              alias={aliases.get(s.id)}
              isCustom={'custom' in s && (s as { custom?: boolean }).custom === true}
              isEditing={editingId === s.id}
              onStartEdit={() => setEditingId(s.id)}
              onStopEdit={() => setEditingId(null)}
              onToggleFavorite={() => toggleFavorite(s.id)}
              onSetAlias={(alias) => setAlias(s.id, alias)}
              onRemove={() => removeCustomSnippet(s.id)}
            />
          ))
        )}
      </div>
    </section>
  );
}

function CategoryPill({ label, active, onClick }: { label: string; active: boolean; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
        active
          ? 'bg-accent/20 text-accent border border-transparent shadow-[inset_0_0_0_1px_color-mix(in_srgb,var(--color-accent)_35%,transparent)]'
          : 'bg-white/[0.03] text-textSecondary border border-transparent hover:bg-white/[0.06] hover:text-textPrimary'
      }`}
    >
      {label}
    </button>
  );
}

// ---------- Snippet row -----------------------------------------------------

interface SnippetRowProps {
  snippet: Snippet;
  isFavorite: boolean;
  alias?: string;
  isCustom: boolean;
  isEditing: boolean;
  onStartEdit: () => void;
  onStopEdit: () => void;
  onToggleFavorite: () => void;
  onSetAlias: (alias: string) => void;
  onRemove: () => void;
}

function SnippetRow({
  snippet,
  isFavorite,
  alias,
  isCustom,
  isEditing,
  onStartEdit,
  onStopEdit,
  onToggleFavorite,
  onSetAlias,
  onRemove,
}: SnippetRowProps) {
  const updateCustomSnippet = useSnippetStore((s) => s.updateCustomSnippet);
  const [aliasDraft, setAliasDraft] = useState(alias ?? '');
  const [contentDraft, setContentDraft] = useState(snippet.content);
  const [titleDraft, setTitleDraft] = useState(snippet.title);

  return (
    <div className="px-4 py-3">
      <div className="flex items-start gap-3">
        <button
          type="button"
          onClick={onToggleFavorite}
          aria-label={isFavorite ? 'Unfavorite' : 'Favorite'}
          className={`mt-0.5 p-1 rounded transition-colors ${
            isFavorite ? 'text-amber-400 hover:text-amber-300' : 'text-textMuted hover:text-textPrimary'
          }`}
        >
          <Star size={16} className={isFavorite ? 'fill-amber-400' : ''} />
        </button>

        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 mb-1 flex-wrap">
            <span className="text-sm font-semibold text-textPrimary truncate">{snippet.title}</span>
            <CategoryBadge category={snippet.category} />
            {isCustom && (
              <span className="rounded bg-violet-400/15 text-violet-300 px-1.5 py-0.5 text-[10px] uppercase tracking-wide font-semibold">
                Custom
              </span>
            )}
          </div>

          {isEditing && isCustom ? (
            <div className="space-y-2 mt-2">
              <label className="block">
                <span className="mb-1 block text-[11px] text-textMuted">Title</span>
                <input
                  value={titleDraft}
                  onChange={(e) => setTitleDraft(e.target.value)}
                  placeholder="Title"
                  className="w-full rounded-md border border-white/10 bg-black/30 px-2.5 py-1.5 text-sm text-textPrimary"
                />
              </label>
              <label className="block">
                <span className="mb-1 block text-[11px] text-textMuted">Content</span>
                <textarea
                  value={contentDraft}
                  onChange={(e) => setContentDraft(e.target.value)}
                  placeholder="Content"
                  rows={3}
                  className="w-full rounded-md border border-white/10 bg-black/30 px-2.5 py-1.5 text-sm text-textPrimary font-mono"
                />
              </label>
            </div>
          ) : (
            <pre className="mt-1 whitespace-pre-wrap text-[12px] text-textSecondary leading-relaxed line-clamp-3 font-sans">
              {snippet.content}
            </pre>
          )}

          <div className="mt-2 flex items-center gap-2 flex-wrap">
            <label className="text-[11px] text-textMuted">Alias:</label>
            <input
              type="text"
              value={aliasDraft}
              onChange={(e) => setAliasDraft(e.target.value)}
              onBlur={() => {
                if (aliasDraft !== (alias ?? '')) onSetAlias(aliasDraft);
              }}
              onKeyDown={(e) => {
                if (e.key === 'Enter') {
                  (e.target as HTMLInputElement).blur();
                }
              }}
              placeholder="(none)"
              className="rounded border border-white/10 bg-black/30 px-2 py-0.5 text-[12px] text-textPrimary font-mono w-32 focus:border-accent/60 focus:outline-none"
            />
            <span className="text-[10px] text-textMuted">
              {alias ? `Typing "${alias}" in the palette boosts this snippet to the top.` : 'Bind a typed shortcut.'}
            </span>
          </div>
        </div>

        <div className="flex items-center gap-1 flex-shrink-0">
          <button
            type="button"
            onClick={() => copyToClipboard(snippet.content, `Copied "${snippet.title}"`)}
            aria-label="Copy"
            className="p-1.5 rounded text-textMuted hover:text-textPrimary hover:bg-white/[0.06] transition-colors"
          >
            <Copy size={14} />
          </button>
          {isCustom && (
            <>
              {isEditing ? (
                <>
                  <button
                    type="button"
                    onClick={() => {
                      updateCustomSnippet(snippet.id, { title: titleDraft, content: contentDraft });
                      onStopEdit();
                    }}
                    aria-label="Save"
                    className="p-1.5 rounded text-emerald-400 hover:text-emerald-300 hover:bg-white/[0.06] transition-colors"
                  >
                    <Save size={14} />
                  </button>
                  <button
                    type="button"
                    onClick={onStopEdit}
                    aria-label="Cancel"
                    className="p-1.5 rounded text-textMuted hover:text-textPrimary hover:bg-white/[0.06] transition-colors"
                  >
                    <XIcon size={14} />
                  </button>
                </>
              ) : (
                <button
                  type="button"
                  onClick={onStartEdit}
                  aria-label="Edit"
                  className="p-1.5 rounded text-textMuted hover:text-textPrimary hover:bg-white/[0.06] transition-colors"
                >
                  <Pencil size={14} />
                </button>
              )}
              <button
                type="button"
                onClick={() => {
                  if (confirm(`Delete "${snippet.title}"?`)) onRemove();
                }}
                aria-label="Delete"
                className="p-1.5 rounded text-rose-400 hover:text-rose-300 hover:bg-white/[0.06] transition-colors"
              >
                <Trash2 size={14} />
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function CategoryBadge({ category }: { category: Snippet['category'] }) {
  return (
    <span className="rounded bg-white/[0.06] text-textSecondary px-1.5 py-0.5 text-[10px] uppercase tracking-wide font-semibold">
      {category}
    </span>
  );
}

// ---------- Add custom snippet ----------------------------------------------

function AddCustomForm({ onDone }: { onDone: () => void }) {
  const addCustomSnippet = useSnippetStore((s) => s.addCustomSnippet);
  const setAlias = useSnippetStore((s) => s.setAlias);
  const [title, setTitle] = useState('');
  const [content, setContent] = useState('');
  const [category, setCategory] = useState<Snippet['category']>('Meme');
  const [alias, setAliasInput] = useState('');

  const submit = () => {
    if (!title.trim() || !content.trim()) {
      useAppStore.getState().addToast('Title and content are required', 'warning');
      return;
    }
    const id = addCustomSnippet({ title, category, content });
    if (alias.trim()) setAlias(id, alias.trim());
    useAppStore.getState().addToast(`Added "${title}"`, 'success');
    setTitle('');
    setContent('');
    setAliasInput('');
    setCategory('Meme');
    onDone();
  };

  return (
    <div className="mb-4 rounded-lg border border-accent/30 bg-accent/[0.04] p-4 space-y-3">
      <div className="flex items-center gap-2">
        <Plus size={14} className="text-accent" />
        <h4 className="text-sm font-semibold text-textPrimary">New custom snippet</h4>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="flex flex-col">
          <span className="mb-1 block text-xs text-textMuted">Title</span>
          <input
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            placeholder="Shown in the palette"
            className="rounded-md border border-white/10 bg-black/30 px-3 py-1.5 text-sm text-textPrimary"
          />
        </label>
        <div>
          <span className="mb-1 block text-xs text-textMuted">Category</span>
          <Dropdown
            value={category}
            onChange={setCategory}
            className="w-full"
            ariaLabel="Snippet category"
            options={SNIPPET_CATEGORIES.map((c) => ({ value: c as Snippet['category'], label: String(c) }))}
          />
        </div>
      </div>
      <label className="block">
        <span className="mb-1 block text-xs text-textMuted">Content</span>
        <textarea
          value={content}
          onChange={(e) => setContent(e.target.value)}
          placeholder="What gets copied to your clipboard when you pick this snippet"
          rows={4}
          className="w-full rounded-md border border-white/10 bg-black/30 px-3 py-2 text-sm text-textPrimary font-mono"
        />
      </label>
      <div className="flex items-center gap-2">
        <label className="text-xs text-textMuted">Alias (optional):</label>
        <input
          value={alias}
          onChange={(e) => setAliasInput(e.target.value)}
          placeholder="e.g. greet"
          className="rounded-md border border-white/10 bg-black/30 px-3 py-1 text-xs text-textPrimary font-mono w-32"
        />
        <span className="text-[10px] text-textMuted">Type this in the palette to instantly match this snippet.</span>
      </div>
      <div className="flex justify-end gap-2">
        <button
          type="button"
          onClick={onDone}
          className="rounded-md px-3 py-1.5 text-sm text-textSecondary hover:text-textPrimary"
        >
          Cancel
        </button>
        <button
          type="button"
          onClick={submit}
          className="rounded-md bg-accent px-3 py-1.5 text-sm font-semibold text-background hover:bg-accent-hover transition-colors"
        >
          Add snippet
        </button>
      </div>
    </div>
  );
}

// ---------- Kbd -------------------------------------------------------------

function Kbd({ children, className = '' }: { children: React.ReactNode; className?: string }) {
  return (
    <kbd
      className={`inline-grid h-5 min-w-[1.5rem] place-items-center rounded border border-white/20 bg-white/[0.06] px-1.5 text-[10px] font-mono font-semibold text-textPrimary ${className}`}
    >
      {children}
    </kbd>
  );
}
