import { useMemo, useState } from 'react';
import { ChevronDown, ChevronUp, Plus, Trash2, AlertTriangle, BookOpen } from 'lucide-react';
import { useAppStore } from '../../stores/AppStore';
import { RESERVED_TRIGGERS } from '../../utils/chatCommands';
import { SettingsSection } from './_primitives';
import type { UserSlashCommand } from '../../types';

function makeDefaultCommand(): UserSlashCommand {
  return {
    id:
      typeof crypto !== 'undefined' && 'randomUUID' in crypto
        ? crypto.randomUUID()
        : Math.random().toString(36).slice(2),
    trigger: '',
    expansion: '',
    description: '',
    enabled: true,
    require_slash: true,
    also_match_suffix: false,
  };
}

// Strip leading slash + non-alphanumeric to keep triggers tidy. Twitch
// commands are all ascii lowercase alnum + a few specials; we restrict to
// alnum to avoid surprises with whitespace, /, !, etc.
function sanitizeTrigger(raw: string): string {
  return raw.replace(/^\//, '').replace(/[^a-zA-Z0-9_]/g, '').toLowerCase();
}

// Inline code chip used throughout the docs panel for placeholder names
// and short snippets. Single visual treatment, no decoration.
const Tag: React.FC<{ children: React.ReactNode }> = ({ children }) => (
  <code className="px-1 py-0.5 bg-background/40 rounded text-textPrimary font-mono text-[11px]">{children}</code>
);

const UserCommandsSettings = () => {
  const { settings, updateSettings } = useAppStore();
  const [showDocs, setShowDocs] = useState<boolean>(false);
  const commands = useMemo(
    () => settings.chat_commands?.user_commands ?? [],
    [settings.chat_commands],
  );

  const writeCommands = (next: UserSlashCommand[]) => {
    updateSettings({
      ...settings,
      chat_commands: { user_commands: next },
    });
  };

  const updateCommand = (id: string, patch: Partial<UserSlashCommand>) => {
    writeCommands(commands.map((c) => (c.id === id ? { ...c, ...patch } : c)));
  };

  const removeCommand = (id: string) => {
    writeCommands(commands.filter((c) => c.id !== id));
  };

  const moveCommand = (id: string, direction: -1 | 1) => {
    const idx = commands.findIndex((c) => c.id === id);
    if (idx === -1) return;
    const target = idx + direction;
    if (target < 0 || target >= commands.length) return;
    const next = commands.slice();
    [next[idx], next[target]] = [next[target], next[idx]];
    writeCommands(next);
  };

  const addCommand = () => {
    writeCommands([...commands, makeDefaultCommand()]);
  };

  // Detect duplicates among user triggers (only the first instance fires).
  const duplicateTriggers = useMemo(() => {
    const seen = new Set<string>();
    const dups = new Set<string>();
    for (const c of commands) {
      const t = c.trigger.toLowerCase();
      if (!t) continue;
      if (seen.has(t)) dups.add(t);
      seen.add(t);
    }
    return dups;
  }, [commands]);

  return (
    <SettingsSection label="Custom Commands" bare>
      <div className="flex items-start justify-between gap-3">
        <p className="text-xs text-textSecondary flex-1">
          Define your own commands. A trigger of <code className="px-1 py-0.5 bg-glass/40 rounded text-textPrimary">lurk</code> with expansion <code className="px-1 py-0.5 bg-glass/40 rounded text-textPrimary">I&apos;m lurking, BRB</code> means typing <code className="px-1 py-0.5 bg-glass/40 rounded text-textPrimary">/lurk</code> sends that message instead.
        </p>
        <button
          onClick={() => setShowDocs((v) => !v)}
          className="inline-flex items-center gap-1.5 px-2 py-1 text-xs text-textSecondary hover:text-textPrimary transition-colors flex-shrink-0"
        >
          <BookOpen size={12} />
          <span>{showDocs ? 'Hide reference' : 'Show reference'}</span>
        </button>
      </div>

      {showDocs && (
        <div className="mb-4 bg-glass/30 rounded-lg p-5 text-xs leading-relaxed space-y-6">

          <section>
            <h4 className="text-textPrimary font-medium mb-2">Words from what they typed</h4>
            <p className="text-textSecondary">
              Anything typed after your command gets numbered. If you make a command <Tag>so</Tag> and someone types <Tag>/so jane is great</Tag>, then <Tag>{'{1}'}</Tag> is <span className="text-textPrimary">jane</span>, <Tag>{'{2}'}</Tag> is <span className="text-textPrimary">is</span>, and <Tag>{'{3}'}</Tag> is <span className="text-textPrimary">great</span>.
            </p>
            <p className="text-textSecondary mt-2">
              For everything from a certain word onward, use <Tag>{'{2+}'}</Tag> (gives <span className="text-textPrimary">is great</span>) or <Tag>{'{*}'}</Tag> (gives <span className="text-textPrimary">jane is great</span>, same as <Tag>{'{1+}'}</Tag>).
            </p>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">Auto-filled info</h4>
            <p className="text-textSecondary mb-3">
              These get replaced automatically. No typing needed.
            </p>
            <dl className="space-y-1.5">
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{user}'}</Tag></dt>
                <dd className="text-textSecondary">your display name</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{user.id}'}</Tag></dt>
                <dd className="text-textSecondary">your numeric Twitch ID</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{channel}'}</Tag></dt>
                <dd className="text-textSecondary">the channel you&apos;re watching</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{channel.id}'}</Tag></dt>
                <dd className="text-textSecondary">that channel&apos;s numeric Twitch ID</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{stream.title}'}</Tag></dt>
                <dd className="text-textSecondary">the streamer&apos;s title</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{stream.game}'}</Tag></dt>
                <dd className="text-textSecondary">the streamer&apos;s category</dd>
              </div>
              <div className="flex gap-3">
                <dt className="min-w-[7.5rem]"><Tag>{'{stream.uptime}'}</Tag></dt>
                <dd className="text-textSecondary">how long they&apos;ve been live, like <span className="text-textPrimary">2h 14m</span></dd>
              </div>
            </dl>
            <p className="text-textMuted mt-3">
              The stream ones come back empty when the channel is offline.
            </p>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">Sending a literal {'{'} or {'}'}</h4>
            <p className="text-textSecondary">
              Double them up. <Tag>{'{{'}</Tag> sends a single <Tag>{'{'}</Tag>, and <Tag>{'}}'}</Tag> sends a single <Tag>{'}'}</Tag>.
            </p>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-3">Examples</h4>

            <div className="space-y-4">
              <div>
                <p className="text-textSecondary">
                  Trigger <Tag>so</Tag> with expansion <Tag>shoutout to @{'{1}'}, go follow them at twitch.tv/{'{1}'}</Tag>.
                </p>
                <p className="text-textSecondary mt-1">
                  Typing <Tag>/so jane</Tag> sends <span className="text-textPrimary">shoutout to @jane, go follow them at twitch.tv/jane</span>.
                </p>
              </div>

              <div>
                <p className="text-textSecondary">
                  Trigger <Tag>title</Tag> with expansion <Tag>{'{stream.title}'} (playing {'{stream.game}'}, live for {'{stream.uptime}'})</Tag>.
                </p>
                <p className="text-textSecondary mt-1">
                  Typing <Tag>/title</Tag> sends the current title, game, and uptime. No words needed after the trigger.
                </p>
              </div>

              <div>
                <p className="text-textSecondary">
                  Trigger <Tag>day</Tag> with expansion <Tag>Have a {'{1+}'} day!</Tag>.
                </p>
                <p className="text-textSecondary mt-1">
                  Typing <Tag>/day really really great</Tag> sends <span className="text-textPrimary">Have a really really great day!</span>
                </p>
              </div>
            </div>
          </section>

          <section className="pt-5 border-t border-borderSubtle/40">
            <h4 className="text-textPrimary font-medium mb-2">The two checkboxes on each command</h4>
            <p className="text-textSecondary">
              <span className="text-textPrimary">Require leading /</span> is on by default. Turn it off to fire the trigger from plain messages with no slash. A trigger of <Tag>lurk</Tag> with this off means just typing <span className="text-textPrimary">lurk</span> sends your message.
            </p>
            <p className="text-textSecondary mt-2">
              <span className="text-textPrimary">Also match at end of message</span> additionally fires when the trigger is the last word of a message. Handy for catchphrase shortcuts at the end of what you&apos;re saying.
            </p>
          </section>

        </div>
      )}

      <div className="space-y-3">
        {commands.length === 0 && (
          <div className="bg-glass/30 rounded-lg px-4 py-6 text-center">
            <p className="text-sm text-textSecondary mb-3">
              No custom commands yet.
            </p>
            <button
              onClick={addCommand}
              className="glass-button inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-textPrimary text-sm font-medium"
            >
              <Plus size={14} />
              Add your first command
            </button>
          </div>
        )}

        {commands.map((command, idx) => {
          const triggerLower = command.trigger.toLowerCase();
          const reservedConflict = triggerLower.length > 0 && RESERVED_TRIGGERS.has(triggerLower);
          const duplicateConflict = duplicateTriggers.has(triggerLower);
          const warning = reservedConflict
            ? `Trigger "/${triggerLower}" is a built-in command and won't fire.`
            : duplicateConflict
              ? `Another command already uses "/${triggerLower}". Only the first will fire.`
              : null;

          return (
            <div key={command.id} className="bg-glass/30 rounded-lg p-3 space-y-2">
              <div className="flex items-center gap-2">
                <button
                  onClick={() => updateCommand(command.id, { enabled: !command.enabled })}
                  className={`relative inline-flex h-5 w-9 items-center rounded-full transition-colors flex-shrink-0 ${
                    command.enabled ? 'bg-accent' : 'bg-gray-600'
                  }`}
                  aria-label={command.enabled ? 'Disable command' : 'Enable command'}
                >
                  <span
                    className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                      command.enabled ? 'translate-x-5' : 'translate-x-1'
                    }`}
                  />
                </button>

                <div className="flex items-center bg-background/40 rounded border border-borderSubtle pl-2 flex-1">
                  <span className="text-textSecondary text-sm select-none">/</span>
                  <input
                    type="text"
                    value={command.trigger}
                    onChange={(e) => updateCommand(command.id, { trigger: sanitizeTrigger(e.target.value) })}
                    placeholder="lurk"
                    aria-label="Trigger"
                    maxLength={32}
                    className="flex-1 bg-transparent text-textPrimary text-sm px-1.5 py-1.5 focus:outline-none"
                    spellCheck={false}
                  />
                </div>

                <div className="flex items-center gap-1 flex-shrink-0">
                  <button
                    onClick={() => moveCommand(command.id, -1)}
                    disabled={idx === 0}
                    className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                    aria-label="Move up"
                  >
                    <ChevronUp size={14} />
                  </button>
                  <button
                    onClick={() => moveCommand(command.id, 1)}
                    disabled={idx === commands.length - 1}
                    className="p-1 text-textSecondary hover:text-textPrimary disabled:opacity-30 disabled:cursor-not-allowed transition-colors"
                    aria-label="Move down"
                  >
                    <ChevronDown size={14} />
                  </button>
                  <button
                    onClick={() => removeCommand(command.id)}
                    className="p-1 text-textSecondary hover:text-red-400 transition-colors"
                    aria-label="Delete command"
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              </div>

              <label className="block">
                <span className="mb-1 block text-[11px] text-textMuted">Message it sends</span>
                <textarea
                  value={command.expansion}
                  onChange={(e) => updateCommand(command.id, { expansion: e.target.value })}
                  placeholder="What this command sends to chat. Supports {1}, {2}, {*}, {user}, {channel}."
                  rows={2}
                  className="w-full glass-input text-textPrimary text-sm px-2.5 py-1.5 resize-y"
                  spellCheck={false}
                />
              </label>

              <label className="block">
                <span className="mb-1 block text-[11px] text-textMuted">Description (optional)</span>
                <input
                  type="text"
                  value={command.description ?? ''}
                  onChange={(e) => updateCommand(command.id, { description: e.target.value })}
                  placeholder="Shown next to the command in the picker"
                  maxLength={120}
                  className="w-full glass-input text-textPrimary text-xs px-2.5 py-1.5"
                  spellCheck={false}
                />
              </label>

              <div className="flex items-center gap-4 pt-1 text-xs text-textSecondary">
                <label className="inline-flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={command.require_slash !== false}
                    onChange={(e) => updateCommand(command.id, { require_slash: e.target.checked })}
                    className="accent-accent"
                  />
                  Require leading <code className="px-1 py-0.5 bg-glass/40 rounded text-textPrimary">/</code>
                </label>
                <label className="inline-flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={command.also_match_suffix === true}
                    onChange={(e) => updateCommand(command.id, { also_match_suffix: e.target.checked })}
                    className="accent-accent"
                  />
                  Also match at end of message
                </label>
              </div>

              {warning && (
                <div className="flex items-start gap-1.5 text-xs text-amber-300">
                  <AlertTriangle size={12} className="mt-0.5 flex-shrink-0" />
                  <span>{warning}</span>
                </div>
              )}
            </div>
          );
        })}

        {commands.length > 0 && (
          <button
            onClick={addCommand}
            className="inline-flex items-center gap-2 px-3 py-1.5 text-textSecondary hover:text-textPrimary text-sm transition-colors"
          >
            <Plus size={14} />
            Add another command
          </button>
        )}
      </div>
    </SettingsSection>
  );
};

export default UserCommandsSettings;
