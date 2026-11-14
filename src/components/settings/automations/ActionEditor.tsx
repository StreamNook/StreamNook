// One action of a rule: a Reply or a Post with its lines, or a Notify with its
// title, text and sound, plus the variables to insert and the live preview.
import { useRef, useState, type RefObject } from 'react';
import { ArrowDown, ArrowUp, ExternalLink, Plus, Trash2, Volume2, X } from 'lucide-react';
import type { AutomationAction, AutomationFieldError, AutomationLine, AutomationPick, AutomationPlatform } from '../../../types';
import { errorAt, newElementId, newLine, type AutomationVariable } from '../../../utils/automations';
import { makeKey } from '../../../utils/providerKey';
import { useAppStore } from '../../../stores/AppStore';
import { BUILT_IN_SOUNDS, SOUND_LABELS, playSound } from '../../../utils/notificationSound';
import { openSystemNotifySettings, useSystemNotifyPermission } from '../../../utils/systemNotifyPermission';
import { IS_MAC, IS_MOBILE } from '../../../utils/platform';
import { Tooltip } from '../../ui/Tooltip';
import { InlineSlider } from '../../plugins/settingsPageKit';
import { ChannelPicker } from '../ChannelPicker';
import { isP2Action } from '../../../utils/automations';
import { ACTION_NAMES, ACTION_WORDS } from './labels';
import { P2ActionFields } from './P2Fields';
import { Disclosure, Field, HINT, INPUT, QUIET_BUTTON, SELECT, SwitchLine } from './parts';

const MAX_LINES = 20;

const PICKS: { value: AutomationPick; label: string }[] = [
  { value: 'random', label: 'A random line' },
  { value: 'shuffle', label: 'Each line once, then again' },
  { value: 'in_order', label: 'In order' },
];

type LinesAction = Extract<AutomationAction, { kind: 'reply' | 'send' }>;
type SendAction = Extract<AutomationAction, { kind: 'send' }>;
type NotifyAction = Extract<AutomationAction, { kind: 'notify' }>;

function LinesField({
  action,
  at,
  errors,
  lineRefs,
  onFocusLine,
  onChange,
}: {
  action: LinesAction;
  at: string;
  errors: readonly AutomationFieldError[];
  lineRefs: RefObject<Map<string, HTMLTextAreaElement>>;
  onFocusLine: (id: string) => void;
  onChange: (next: AutomationAction) => void;
}) {
  const setLines = (texts: AutomationLine[]) => onChange({ ...action, texts });
  const many = action.texts.length > 1;
  return (
    <Field
      label={many ? 'Messages' : 'Message'}
      hint={many ? 'One is picked each time it acts.' : undefined}
      error={errorAt(errors, `${at}.texts`)}
    >
      <div className="space-y-2">
        {action.texts.map((line, j) => {
          const lineError = errorAt(errors, `${at}.texts[${j}].text`);
          return (
            <div key={line.id} className="space-y-1">
              <div className="flex items-start gap-2">
                <textarea
                  ref={(el) => {
                    if (el) lineRefs.current.set(line.id, el);
                    else lineRefs.current.delete(line.id);
                  }}
                  value={line.text}
                  rows={2}
                  maxLength={500}
                  aria-label={many ? `Message ${j + 1}` : 'Message'}
                  onFocus={() => onFocusLine(line.id)}
                  onChange={(e) =>
                    setLines(action.texts.map((l) => (l.id === line.id ? { ...l, text: e.target.value } : l)))
                  }
                  placeholder={action.kind === 'reply' ? '@{user} thanks for stopping by!' : 'Welcome in, everyone!'}
                  className={`${INPUT} resize-y`}
                />
                {many && (
                  <button
                    type="button"
                    onClick={() => setLines(action.texts.filter((l) => l.id !== line.id))}
                    className="mt-1.5 rounded p-1 text-textMuted transition-colors hover:text-textPrimary"
                    aria-label="Remove this line"
                  >
                    <X size={13} />
                  </button>
                )}
              </div>
              {lineError && <p className="text-[12px] text-error">{lineError}</p>}
            </div>
          );
        })}
        {action.texts.length < MAX_LINES && (
          <button type="button" onClick={() => setLines([...action.texts, newLine()])} className={QUIET_BUTTON}>
            <Plus size={13} />
            Add another line
          </button>
        )}
      </div>
    </Field>
  );
}

function SendWhere({
  action,
  platform,
  onChange,
}: {
  action: SendAction;
  /** The platform the rule posts on: the other chat is on it. */
  platform: AutomationPlatform;
  onChange: (next: AutomationAction) => void;
}) {
  const to = action.to;
  return (
    <Field label="Where">
      <select
        value={to ? 'other' : 'here'}
        aria-label="Where"
        onChange={(e) =>
          onChange({
            ...action,
            to: e.target.value === 'other' ? (to ?? { id: newElementId('c'), key: '', room_id: '', name: '' }) : null,
          })
        }
        className={SELECT}
      >
        <option value="here">The chat it happened in</option>
        <option value="other">Another chat</option>
      </select>
      {to && (
        <div className="pt-1">
          <ChannelPicker
            login={to.name || undefined}
            label="Choose the chat"
            providers={[platform]}
            onPick={(login, id, provider) =>
              onChange({
                ...action,
                to: {
                  id: to.id || newElementId('c'),
                  // Twitch's key is built in Rust; another platform's names it here.
                  key: provider === 'twitch' ? '' : makeKey(provider, login),
                  room_id: id,
                  name: login,
                },
              })
            }
          />
        </div>
      )}
    </Field>
  );
}

function NotifyFields({
  action,
  at,
  errors,
  onChange,
}: {
  action: NotifyAction;
  at: string;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationAction) => void;
}) {
  const { permission, ask } = useSystemNotifyPermission();
  const customSounds = useAppStore((s) => s.settings.chat_highlights?.custom_sounds);
  const toggleSystem = () => {
    const next = !action.system;
    onChange({ ...action, system: next });
    // A Mac asks once, the first time a system notification is wanted.
    if (next && IS_MAC && permission === 'default') void ask();
  };
  return (
    <div className="space-y-3">
      <Field label="Title" error={errorAt(errors, `${at}.title`)}>
        <input
          value={action.title}
          maxLength={100}
          aria-label="Title"
          onChange={(e) => onChange({ ...action, title: e.target.value })}
          placeholder="{user} mentioned you"
          className={INPUT}
        />
      </Field>
      <Field label="Text" error={errorAt(errors, `${at}.body`)}>
        <textarea
          value={action.body}
          rows={2}
          maxLength={500}
          aria-label="Text"
          onChange={(e) => onChange({ ...action, body: e.target.value })}
          placeholder="{message}"
          className={`${INPUT} resize-y`}
        />
      </Field>
      <SwitchLine
        label="In the app"
        description="A card in StreamNook, with its sound."
        on={action.toast}
        onChange={() => onChange({ ...action, toast: !action.toast })}
      />
      {!IS_MOBILE && (
        <SwitchLine
          label="System notification"
          description="Shows outside StreamNook while it isn't on screen."
          on={action.system}
          onChange={toggleSystem}
        />
      )}
      {!IS_MOBILE && action.system && IS_MAC && permission === 'denied' && (
        <div className="flex items-center justify-between gap-4 rounded-lg border border-warning/30 bg-warning/5 py-2.5 pl-4 pr-3">
          <p className="text-[12px] leading-relaxed text-textSecondary">
            Notifications are turned off for StreamNook in macOS settings, so these will not reach you outside StreamNook until you allow them there.
          </p>
          <button
            type="button"
            onClick={openSystemNotifySettings}
            className="glass-button flex shrink-0 items-center gap-1.5 whitespace-nowrap rounded-lg px-3 py-1.5 text-xs font-medium text-textPrimary"
          >
            Open macOS settings
            <ExternalLink size={12} />
          </button>
        </div>
      )}
      <Field label="Sound">
        <div className="flex flex-wrap items-center gap-2">
          <select
            value={action.sound ?? ''}
            aria-label="Sound"
            onChange={(e) => onChange({ ...action, sound: e.target.value === '' ? null : e.target.value })}
            className={SELECT}
          >
            <option value="">None</option>
            {BUILT_IN_SOUNDS.map((id) => (
              <option key={id} value={id}>
                {SOUND_LABELS[id]}
              </option>
            ))}
            {(customSounds ?? []).map((s) => (
              <option key={s.id} value={s.id}>
                {s.name || 'Custom sound'}
              </option>
            ))}
          </select>
          {action.sound && (
            <button
              type="button"
              onClick={() => playSound(action.sound, Math.round(action.volume * 100))}
              className={QUIET_BUTTON}
            >
              <Volume2 size={13} />
              Play
            </button>
          )}
        </div>
      </Field>
      {action.sound && (
        <Field label="Volume">
          <InlineSlider
            value={Math.round(action.volume * 100)}
            min={0}
            max={100}
            step={5}
            label="Volume"
            format={(v) => `${v}%`}
            onChange={(v) => onChange({ ...action, volume: v / 100 })}
          />
        </Field>
      )}
    </div>
  );
}

type SpeakAction = Extract<AutomationAction, { kind: 'speak' }>;
type HighlightAction = Extract<AutomationAction, { kind: 'highlight' }>;

const MAX_SPEAK = 300;

/** The swatches the Highlight picker offers; the color input takes any other. */
const HIGHLIGHT_SWATCHES = ['#f97316', '#eab308', '#22c55e', '#06b6d4', '#3b82f6', '#a855f7', '#ec4899', '#ef4444'];

function SpeakFields({
  action,
  at,
  errors,
  onChange,
}: {
  action: SpeakAction;
  at: string;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationAction) => void;
}) {
  return (
    <>
      <Field label="Say" hint="Read out loud on this device. It is never posted to chat." error={errorAt(errors, `${at}.text`)}>
        <textarea
          value={action.text}
          rows={2}
          maxLength={MAX_SPEAK}
          aria-label="Say"
          onChange={(e) => onChange({ ...action, text: e.target.value })}
          placeholder="{user} just followed"
          className={`${INPUT} resize-y`}
        />
      </Field>
      <SwitchLine
        label="Also while streamer mode is on"
        description="Streamer mode keeps it quiet unless you turn this on."
        on={action.on_stream}
        onChange={() => onChange({ ...action, on_stream: !action.on_stream })}
      />
    </>
  );
}

function HighlightFields({
  action,
  at,
  errors,
  onChange,
}: {
  action: HighlightAction;
  at: string;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationAction) => void;
}) {
  const color = action.color.toLowerCase();
  const valid = /^#[0-9a-f]{6}$/.test(color);
  return (
    <Field label="Color" hint="Tints the messages this rule matches." error={errorAt(errors, `${at}.color`)}>
      <div className="flex flex-wrap items-center gap-2">
        {HIGHLIGHT_SWATCHES.map((swatch) => (
          <button
            key={swatch}
            type="button"
            aria-label={`Color ${swatch}`}
            aria-pressed={color === swatch}
            onClick={() => onChange({ ...action, color: swatch })}
            className={`h-6 w-6 rounded-full border transition-shadow ${
              color === swatch ? 'border-textPrimary ring-2 ring-inset ring-background' : 'border-borderSubtle'
            }`}
            style={{ backgroundColor: swatch }}
          />
        ))}
        <input
          type="color"
          value={valid ? color : '#f97316'}
          aria-label="Custom color"
          onChange={(e) => onChange({ ...action, color: e.target.value.toLowerCase() })}
          className="h-6 w-6 cursor-pointer overflow-hidden rounded-full border border-borderSubtle bg-transparent p-0 [&::-webkit-color-swatch-wrapper]:p-0 [&::-webkit-color-swatch]:rounded-full [&::-webkit-color-swatch]:border-0"
        />
      </div>
    </Field>
  );
}

interface ActionEditorProps {
  action: AutomationAction;
  index: number;
  count: number;
  /** Badge rules post in your channel and never reply. */
  badgeRule: boolean;
  /** The platform the rule posts on (a Send's other chat is on it). */
  platform: AutomationPlatform;
  /** A Timer or Clock rule in your own channel: the command forms that need it are offered. */
  timedOwn: boolean;
  /** Every error of the rule; this picks its own by path. */
  errors: readonly AutomationFieldError[];
  variables: readonly AutomationVariable[];
  /** The text this action would send, from the live preview. */
  preview: string | null;
  onChange: (next: AutomationAction) => void;
  onRemove: () => void;
  onMove: (dir: -1 | 1) => void;
}

export function ActionEditor({
  action,
  index,
  count,
  badgeRule,
  timedOwn,
  platform,
  errors,
  variables,
  preview,
  onChange,
  onRemove,
  onMove,
}: ActionEditorProps) {
  const lineRefs = useRef(new Map<string, HTMLTextAreaElement>());
  const [focusLine, setFocusLine] = useState<string | null>(null);
  const at = `actions[${index}]`;
  const ownError = errorAt(errors, at);
  const shown = variables.filter((v) => badgeRule || v.scope !== 'badge');

  // A variable goes where the caret was in the line last typed in (the first
  // line before any), or at the end of a Notify's text.
  const insert = (name: string) => {
    const token = `{${name}}`;
    if (isP2Action(action)) return;
    if (action.kind === 'notify') {
      onChange({ ...action, body: `${action.body}${token}` });
      return;
    }
    if (action.kind === 'speak') {
      onChange({ ...action, text: `${action.text}${token}` });
      return;
    }
    if (action.kind === 'highlight') return;
    const lineId = focusLine && action.texts.some((l) => l.id === focusLine) ? focusLine : action.texts[0]?.id;
    const line = action.texts.find((l) => l.id === lineId);
    if (!line) return;
    const el = lineRefs.current.get(line.id);
    const start = el?.selectionStart ?? line.text.length;
    const end = el?.selectionEnd ?? start;
    const text = `${line.text.slice(0, start)}${token}${line.text.slice(end)}`;
    onChange({ ...action, texts: action.texts.map((l) => (l.id === line.id ? { ...l, text } : l)) });
  };

  return (
    <div
      className={
        IS_MOBILE
          ? 'space-y-3 border-t border-borderSubtle/60 pt-3'
          : 'space-y-3 rounded-md bg-textPrimary/[0.04] p-3'
      }
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-[13px] font-medium text-textPrimary">{ACTION_NAMES[action.kind]}</span>
        <div className="flex items-center gap-0.5">
          <button
            type="button"
            onClick={() => onMove(-1)}
            disabled={index === 0}
            className={`rounded ${IS_MOBILE ? 'p-2.5' : 'p-1'} text-textSecondary transition-colors hover:text-textPrimary disabled:cursor-not-allowed disabled:opacity-30`}
            aria-label="Move up"
          >
            <ArrowUp size={13} />
          </button>
          <button
            type="button"
            onClick={() => onMove(1)}
            disabled={index === count - 1}
            className={`rounded ${IS_MOBILE ? 'p-2.5' : 'p-1'} text-textSecondary transition-colors hover:text-textPrimary disabled:cursor-not-allowed disabled:opacity-30`}
            aria-label="Move down"
          >
            <ArrowDown size={13} />
          </button>
          <button
            type="button"
            onClick={onRemove}
            disabled={count === 1}
            className="rounded p-1 text-textSecondary transition-colors hover:text-error disabled:cursor-not-allowed disabled:opacity-30"
            aria-label="Remove this action"
          >
            <Trash2 size={13} />
          </button>
        </div>
      </div>
      {ownError && <p className="text-[12px] text-error">{ownError}</p>}

      {action.kind === 'notify' ? (
        <NotifyFields action={action} at={at} errors={errors} onChange={onChange} />
      ) : action.kind === 'speak' ? (
        <SpeakFields action={action} at={at} errors={errors} onChange={onChange} />
      ) : action.kind === 'highlight' ? (
        <HighlightFields action={action} at={at} errors={errors} onChange={onChange} />
      ) : isP2Action(action) ? (
        <P2ActionFields
          action={action}
          at={at}
          errors={errors}
          timedOwn={timedOwn}
          onChange={onChange}
        />
      ) : (
        <>
          {action.kind === 'send' && !badgeRule && <SendWhere action={action} platform={platform} onChange={onChange} />}
          <LinesField
            action={action}
            at={at}
            errors={errors}
            lineRefs={lineRefs}
            onFocusLine={setFocusLine}
            onChange={onChange}
          />
          {action.texts.length > 1 && (
            <Field label="Pick">
              <select
                value={action.pick}
                aria-label="Pick"
                onChange={(e) => onChange({ ...action, pick: e.target.value as AutomationPick })}
                className={SELECT}
              >
                {PICKS.map((p) => (
                  <option key={p.value} value={p.value}>
                    {p.label}
                  </option>
                ))}
              </select>
            </Field>
          )}
          {action.kind === 'reply' && (
            <SwitchLine
              label="Reply in thread"
              description="Shows as a reply to their message."
              on={action.thread}
              onChange={() => onChange({ ...action, thread: !action.thread })}
            />
          )}
        </>
      )}

      {shown.length > 0 && !isP2Action(action) && action.kind !== 'highlight' && (
        <Disclosure label="Insert a variable">
          <div className="flex flex-wrap gap-1.5">
            {shown.map((v) => (
              <Tooltip
                key={v.names[0]}
                content={
                  <span className="block max-w-[32ch] text-left leading-relaxed">
                    <span className="block">{v.value}</span>
                    <span className="block text-textMuted">{v.allowed}</span>
                  </span>
                }
              >
                <button
                  type="button"
                  // Keeps the line's caret where it was.
                  onMouseDown={(e) => e.preventDefault()}
                  onClick={() => insert(v.names[0])}
                  className="rounded-md border border-borderSubtle bg-background/40 px-1.5 py-0.5 font-mono text-[11.5px] text-textSecondary transition-colors hover:text-textPrimary"
                >
                  {`{${v.names[0]}}`}
                </button>
              </Tooltip>
            ))}
          </div>
          <p className={HINT}>Hover a variable to see what it fills in and where it may go.</p>
        </Disclosure>
      )}

      {preview && (
        <p className="rounded-md bg-background/50 px-2.5 py-2 text-[12px] leading-relaxed text-textSecondary">
          <span className="mr-1.5 text-textMuted">{ACTION_WORDS[action.kind]}</span>
          {preview}
        </p>
      )}
    </div>
  );
}
