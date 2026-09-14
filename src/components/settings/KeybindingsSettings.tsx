// KeybindingsSettings — the customizable hotkey editor.
//
// Lists every bindable command grouped by category, with a live search box, a
// key recorder for assigning/clearing combos, per-command and global reset, and
// same-context conflict warnings. Reads/writes through src/keybindings so the
// command palette and the global dispatcher always reflect changes instantly.

import { useEffect, useMemo, useRef, useState } from 'react';
import { Keyboard, RotateCcw, Plus, X as XIcon, Lock } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { useAppStore } from '../../stores/AppStore';
import {
  getBindableCommands,
  getEffectiveBindings,
  canonicalToDisplay,
  addBinding,
  removeBinding,
  resetBinding,
  resetAllBindings,
  findConflicts,
  isCustomized,
  setRecordingKeybind,
  eventToChord,
  chordToCanonical,
  isModifierEvent,
  isCompleteChord,
} from '../../keybindings';
import type { BindableCommand, KeybindCategory, BindingConflict } from '../../keybindings';
import { SettingsSection } from './_primitives';

// Keys the WebView / OS may intercept before the app sees them, so binding
// them is unreliable. We still allow it (the user may know their setup) but warn.
const DISCOURAGED_BINDINGS = new Set(['F12', 'F5', 'Ctrl+R', 'Ctrl+Shift+R', 'Ctrl+Shift+I']);

const CATEGORY_ORDER: { id: KeybindCategory; description: string }[] = [
  { id: 'Application', description: 'Shortcuts that work anywhere in StreamNook.' },
  { id: 'Navigation', description: 'Jump between the main screens of StreamNook without reaching for the mouse.' },
  { id: 'Player', description: 'Control playback while a stream or VOD is playing.' },
  { id: 'Moderation', description: 'Act on chat in channels you moderate: move the focus with J and K, then press an action key on the focused message.' },
  { id: 'Chat', description: 'Shortcuts inside the chat box. These are fixed for now and cannot be rebound.' },
  { id: 'Multi-view', description: 'Shortcuts inside MultiChat windows. These are fixed for now and cannot be rebound.' },
];

export default function KeybindingsSettings() {
  // Subscribe to the override map so every row re-renders the instant a bind
  // changes (also picks up changes made from other windows via the broadcast).
  const overridesRaw = useAppStore((s) => s.settings.keybindings);
  const overrides = useMemo(() => overridesRaw ?? {}, [overridesRaw]);
  const [query, setQuery] = useState('');
  const [recordingFor, setRecordingFor] = useState<string | null>(null);

  const commands = useMemo(() => getBindableCommands(), []);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return commands;
    return commands.filter((cmd) => {
      const binds = getEffectiveBindings(cmd, overrides).map(canonicalToDisplay).join(' ').toLowerCase();
      return (
        cmd.label.toLowerCase().includes(q) ||
        cmd.category.toLowerCase().includes(q) ||
        (cmd.description ?? '').toLowerCase().includes(q) ||
        (cmd.keywords ?? '').toLowerCase().includes(q) ||
        binds.includes(q)
      );
    });
  }, [commands, query, overrides]);

  const byCategory = useMemo(() => {
    const map = new Map<KeybindCategory, BindableCommand[]>();
    for (const cmd of filtered) {
      const list = map.get(cmd.category) ?? [];
      list.push(cmd);
      map.set(cmd.category, list);
    }
    return map;
  }, [filtered]);

  const customizedCount = commands.filter((c) => isCustomized(c, overrides)).length;

  return (
    <div className="space-y-8">
      <Hero customizedCount={customizedCount} />

      {/* Search */}
      <div className="relative">
        <Keyboard
          size={14}
          className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-textMuted"
        />
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search shortcuts by name, action, category, or key…"
          aria-label="Search shortcuts"
          className="w-full rounded-lg border border-white/10 bg-black/30 py-2 pl-9 pr-9 text-sm text-textPrimary placeholder:text-textMuted focus:border-accent/60 focus:outline-none"
        />
        {query && (
          <button
            type="button"
            onClick={() => setQuery('')}
            className="absolute right-2.5 top-1/2 -translate-y-1/2 rounded p-1 text-textMuted hover:text-textPrimary"
            aria-label="Clear search"
          >
            <XIcon size={13} />
          </button>
        )}
      </div>

      {CATEGORY_ORDER.map(({ id, description }) => {
        const list = byCategory.get(id);
        if (!list || list.length === 0) return null;
        return (
          <SettingsSection key={id} label={id} description={description} bare>
            <div className="settings-card hairline-y px-0">
              {list.map((cmd) => (
                <CommandRow
                  key={cmd.id}
                  cmd={cmd}
                  bindings={getEffectiveBindings(cmd, overrides)}
                  customized={isCustomized(cmd, overrides)}
                  recording={recordingFor === cmd.id}
                  onStartRecord={() => setRecordingFor(cmd.id)}
                  onStopRecord={() => setRecordingFor(null)}
                />
              ))}
            </div>
          </SettingsSection>
        );
      })}

      {filtered.length === 0 && (
        <div className="rounded-lg border border-white/5 bg-white/[0.02] px-4 py-10 text-center text-sm text-textMuted">
          No shortcuts match "{query.trim()}".
        </div>
      )}
    </div>
  );
}

// ---------- Hero ------------------------------------------------------------

function Hero({ customizedCount }: { customizedCount: number }) {
  return (
    <div className="rounded-xl border border-white/5 bg-gradient-to-br from-white/[0.04] to-white/[0.01] p-5">
      <p className="text-sm leading-relaxed text-textSecondary">
        Every action below can be triggered by a keyboard shortcut, found in the{' '}
        <Kbd>Ctrl</Kbd> + <Kbd>K</Kbd> command palette, or both. Click{' '}
        <span className="font-medium text-textPrimary">Add</span> on any row to record a new combo,
        remove a chip to clear one, or reset a row to its default. Player shortcuts work while a
        stream is playing; single-key shortcuts never fire while you are typing in chat.
      </p>
      <div className="mt-4 flex items-center gap-3">
        <button
          type="button"
          onClick={() => {
            if (customizedCount === 0) return;
            if (confirm('Reset all keyboard shortcuts to their defaults?')) {
              void resetAllBindings();
            }
          }}
          disabled={customizedCount === 0}
          className="glass-button-secondary inline-flex items-center gap-2 px-3 py-2 text-sm font-medium text-textSecondary transition-colors enabled:hover:text-textPrimary disabled:opacity-40"
        >
          <RotateCcw size={14} /> Reset all to defaults
        </button>
        {customizedCount > 0 && (
          <span className="text-[12px] text-textMuted">
            {customizedCount} shortcut{customizedCount === 1 ? '' : 's'} customized
          </span>
        )}
      </div>
    </div>
  );
}

// ---------- Command row -----------------------------------------------------

interface CommandRowProps {
  cmd: BindableCommand;
  bindings: string[];
  customized: boolean;
  recording: boolean;
  onStartRecord: () => void;
  onStopRecord: () => void;
}

function CommandRow({
  cmd,
  bindings,
  customized,
  recording,
  onStartRecord,
  onStopRecord,
}: CommandRowProps) {
  return (
    <div className="flex items-start justify-between gap-4 px-4 py-3">
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="text-[13px] font-medium text-textPrimary">{cmd.label}</span>
          {cmd.reserved && (
            <span className="inline-flex items-center gap-1 rounded bg-white/[0.06] px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-textMuted">
              <Lock size={9} /> Reserved
            </span>
          )}
        </div>
        {cmd.description && (
          <div className="mt-0.5 text-[12px] leading-relaxed text-textMuted">{cmd.description}</div>
        )}
      </div>

      <div className="flex flex-shrink-0 flex-col items-end gap-2">
        {recording ? (
          <Recorder cmd={cmd} onDone={onStopRecord} />
        ) : (
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            {bindings.length === 0 ? (
              <span className="text-[11px] italic text-textMuted">Not bound</span>
            ) : (
              bindings.map((b) => (
                <span key={b} className="inline-flex items-center gap-1">
                  <KeyCombo chord={b} />
                  {!cmd.reserved && (
                    <button
                      type="button"
                      onClick={() => void removeBinding(cmd.id, b)}
                      className="rounded p-0.5 text-textMuted hover:text-rose-300"
                      aria-label={`Remove ${canonicalToDisplay(b)}`}
                    >
                      <XIcon size={11} />
                    </button>
                  )}
                </span>
              ))
            )}
            {!cmd.reserved && (
              <>
                <button
                  type="button"
                  onClick={onStartRecord}
                  className="inline-flex items-center gap-1 rounded-md border border-transparent bg-white/[0.04] px-2 py-1 text-[11px] font-semibold text-textSecondary shadow-[inset_1px_1px_0_0_rgba(255,255,255,0.10),inset_-1px_-1px_0_0_rgba(0,0,0,0.18)] transition-colors hover:bg-white/[0.07] hover:text-textPrimary"
                >
                  <Plus size={11} /> Add
                </button>
                {customized && (
                  <Tooltip content="Reset to default">
                    <button
                      type="button"
                      onClick={() => void resetBinding(cmd.id)}
                      className="rounded p-1 text-textMuted hover:text-textPrimary"
                    >
                      <RotateCcw size={12} />
                    </button>
                  </Tooltip>
                )}
              </>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

// ---------- Recorder --------------------------------------------------------

function Recorder({ cmd, onDone }: { cmd: BindableCommand; onDone: () => void }) {
  const [pending, setPending] = useState<string | null>(null);
  const [conflicts, setConflicts] = useState<BindingConflict[]>([]);
  const doneRef = useRef(onDone);
  useEffect(() => {
    doneRef.current = onDone;
  });

  // While recording, take over the keyboard: tell the global dispatcher to
  // stand down and capture keys here. Esc cancels; the first complete chord
  // (modifiers + a real key) is captured for confirmation.
  useEffect(() => {
    setRecordingKeybind(true);
    (document.activeElement as HTMLElement | null)?.blur();

    const onKeyDown = (e: KeyboardEvent) => {
      if (e.isComposing || e.keyCode === 229) return; // let IME composition through
      e.preventDefault();
      e.stopPropagation();
      if (e.key === 'Escape') {
        doneRef.current();
        return;
      }
      if (isModifierEvent(e)) return; // wait for a real key
      const chord = eventToChord(e);
      if (!isCompleteChord(chord)) return;
      const canonical = chordToCanonical(chord);
      setPending(canonical);
      setConflicts(findConflicts(canonical, cmd.context, cmd.id));
    };

    window.addEventListener('keydown', onKeyDown, true);
    return () => {
      window.removeEventListener('keydown', onKeyDown, true);
      setRecordingKeybind(false);
    };
  }, [cmd.id, cmd.context]);

  const save = () => {
    if (pending) void addBinding(cmd.id, pending);
    onDone();
  };

  return (
    <div className="flex flex-col items-end gap-1.5">
      <div className="flex items-center gap-2">
        <div className="flex min-w-[120px] items-center justify-center gap-1 rounded-md border border-accent/40 bg-accent/[0.06] px-3 py-1.5">
          {pending ? (
            <KeyCombo chord={pending} />
          ) : (
            <span className="text-[12px] font-medium text-textSecondary">Press keys…</span>
          )}
        </div>
        {pending && (
          <button
            type="button"
            onClick={save}
            className="rounded-md bg-accent px-2.5 py-1.5 text-[11px] font-semibold text-background hover:bg-accent-hover"
          >
            Save
          </button>
        )}
        <button
          type="button"
          onClick={onDone}
          className="glass-button-secondary px-2.5 py-1.5 text-[11px] font-medium text-textSecondary hover:text-textPrimary"
        >
          Cancel
        </button>
      </div>
      {pending && conflicts.length > 0 && (
        <div className="max-w-[260px] text-right text-[11px] leading-snug text-amber-300/90">
          Also used by {conflicts.map((c) => c.label).join(', ')}. Both will fire.
        </div>
      )}
      {pending && DISCOURAGED_BINDINGS.has(pending) && (
        <div className="max-w-[260px] text-right text-[11px] leading-snug text-amber-300/90">
          This key can be intercepted by the app window and may not fire reliably.
        </div>
      )}
    </div>
  );
}

// ---------- Kbd -------------------------------------------------------------

function Kbd({ children }: { children: React.ReactNode }) {
  return <kbd className="sn-keycap">{children}</kbd>;
}

// Renders a chord (e.g. "Ctrl+Shift+D") as separate physical keycaps joined by
// "+", the way a real keyboard combo reads.
function KeyCombo({ chord }: { chord: string }) {
  const parts = chord.split('+');
  return (
    <span className="inline-flex items-center gap-1">
      {parts.map((p, i) => (
        <span key={`${p}-${i}`} className="inline-flex items-center gap-1">
          {i > 0 && <span className="text-[10px] text-textMuted">+</span>}
          <kbd className="sn-keycap">{p === 'Meta' ? '⌘' : p}</kbd>
        </span>
      ))}
    </span>
  );
}
