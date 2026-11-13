// The P3 triggers' fields: a reward picker for "Someone redeems a reward",
// the live or offline choice for "A channel goes live or offline", and the If
// block of every P3 rule (their channel is yours, or the ones a live change names).
import { useEffect, useState } from 'react';
import { X } from 'lucide-react';
import type { AutomationConditions, AutomationFieldError, AutomationRule, AutomationTrigger } from '../../../types';
import { ChannelPicker } from '../ChannelPicker';
import { automationsApi, errorAt, errorText, rewardLabel, type OwnReward } from '../../../utils/automations';
import { Disclosure, Field, HINT, NamesInput, QUIET_BUTTON, SELECT, SwitchLine } from './parts';

const MAX_CHATS = 25;

type RewardTrigger = Extract<AutomationTrigger, { kind: 'reward_redeemed' }>;
type LiveTrigger = Extract<AutomationTrigger, { kind: 'live_change' }>;

/** The words under a P3 trigger with nothing to pick. */
const OWN_HINTS: Partial<Record<AutomationTrigger['kind'], string>> = {
  follow: 'Acts on each new follower of your channel.',
  poll_started: 'Acts when you start a poll in your channel.',
  prediction_started: 'Acts when you start a prediction in your channel.',
};

type RewardState =
  | { status: 'loading' }
  | { status: 'ready'; rewards: OwnReward[] }
  | { status: 'error'; message: string };

function RewardPicker({
  trigger,
  ownerId,
  errors,
  onChange,
}: {
  trigger: RewardTrigger;
  ownerId: string;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationTrigger) => void;
}) {
  const [state, setState] = useState<RewardState>({ status: 'loading' });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let live = true;
    automationsApi
      .ownRewards(ownerId || null)
      .then((rewards) => {
        if (live) setState({ status: 'ready', rewards });
      })
      .catch((e: unknown) => {
        if (live) setState({ status: 'error', message: errorText(e) });
      });
    return () => {
      live = false;
    };
  }, [ownerId, attempt]);

  const chosen = trigger.reward_id ?? '';
  const rewards = state.status === 'ready' ? state.rewards : [];
  const missing = chosen !== '' && !rewards.some((r) => r.id === chosen);
  return (
    <Field
      label="Reward"
      hint={
        state.status === 'ready' && rewards.length === 0
          ? 'You have no rewards yet. Any reward still works.'
          : 'One reward, or any reward.'
      }
      error={errorAt(errors, 'when.reward_id')}
    >
      <div className="flex flex-wrap items-center gap-2">
        <select
          value={chosen}
          aria-label="Reward"
          disabled={state.status === 'loading'}
          onChange={(e) => onChange({ ...trigger, reward_id: e.target.value === '' ? null : e.target.value })}
          className={SELECT}
        >
          <option value="">{state.status === 'loading' ? 'Loading your rewards' : 'Any reward'}</option>
          {missing && <option value={chosen}>{state.status === 'ready' ? 'A reward that is not in your list' : 'Your chosen reward'}</option>}
          {rewards.map((r) => (
            <option key={r.id} value={r.id}>
              {rewardLabel(r)}
            </option>
          ))}
        </select>
        {state.status === 'error' && (
          <button
            type="button"
            onClick={() => {
              setState({ status: 'loading' });
              setAttempt((n) => n + 1);
            }}
            className={QUIET_BUTTON}
          >
            Try again
          </button>
        )}
      </div>
      {state.status === 'error' && <p className="text-[12px] leading-relaxed text-textSecondary">{state.message}</p>}
    </Field>
  );
}

/** When fields of the four own-channel triggers (follow, reward, poll, prediction). */
export function OwnEventFields({
  trigger,
  ownerId,
  errors,
  onChange,
}: {
  trigger: AutomationTrigger;
  ownerId: string;
  errors: readonly AutomationFieldError[];
  onChange: (next: AutomationTrigger) => void;
}) {
  if (trigger.kind === 'reward_redeemed') {
    // Keyed by the owner, so another account's rewards load from a fresh "loading" (no setState in an effect).
    return <RewardPicker key={ownerId} trigger={trigger} ownerId={ownerId} errors={errors} onChange={onChange} />;
  }
  const hint = OWN_HINTS[trigger.kind];
  return hint ? <p className={HINT}>{hint}</p> : null;
}

/** "A channel goes live or offline": which of the two. */
export function LiveChangeFields({ trigger, onChange }: { trigger: LiveTrigger; onChange: (next: AutomationTrigger) => void }) {
  return (
    <Field label="When it">
      <select
        value={trigger.online ? 'online' : 'offline'}
        aria-label="When it"
        onChange={(e) => onChange({ ...trigger, online: e.target.value === 'online' })}
        className={SELECT}
      >
        <option value="online">Goes live</option>
        <option value="offline">Goes offline</option>
      </select>
    </Field>
  );
}

/** The If block of a P3 rule: your own channel (with a live switch and who to skip), or the channels a live change watches. */
export function EdgeIf({
  ownEvent,
  draft,
  errors,
  addChannel,
  setIf,
}: {
  ownEvent: boolean;
  draft: AutomationRule;
  errors: readonly AutomationFieldError[];
  addChannel: (login: string, roomId: string) => void;
  setIf: (patch: Partial<AutomationConditions>) => void;
}) {
  const scope = draft.only_if.channels;
  if (ownEvent) {
    return (
      <>
        <p className={HINT}>In your channel, as it happens.</p>
        <SwitchLine
          label="Only while live"
          description="Only while your stream is live."
          on={draft.only_if.only_live}
          onChange={() => setIf({ only_live: !draft.only_if.only_live })}
        />
        <Disclosure label="More conditions">
          <Field label="Skip these people" hint="Names separated by commas.">
            <NamesInput
              values={draft.only_if.skip_users}
              label="Skip these people"
              placeholder="nightbot, streamelements"
              onCommit={(skip_users) => setIf({ skip_users })}
            />
          </Field>
          <Field label="Only these people" hint="Leave it empty to answer anyone.">
            <NamesInput
              values={draft.only_if.only_users}
              label="Only these people"
              onCommit={(only_users) => setIf({ only_users })}
            />
          </Field>
        </Disclosure>
      </>
    );
  }
  const list = scope.kind === 'these' ? scope.list : [];
  return (
    <Field label="Channels" hint="It acts once for each broadcast of each channel you add." error={errorAt(errors, 'only_if.channels')}>
      <div className="space-y-1.5">
        {list.map((c, i) => {
          const chatError = errorAt(errors, `only_if.channels.list[${i}]`);
          return (
            <div key={c.id} className="space-y-1">
              <div className="flex items-center justify-between gap-2 rounded-md bg-background/40 px-2.5 py-1.5">
                <span className="truncate text-[13px] text-textPrimary">#{c.name || c.key}</span>
                <button
                  type="button"
                  onClick={() => setIf({ channels: { kind: 'these', list: list.filter((x) => x.id !== c.id) } })}
                  className="rounded p-0.5 text-textMuted transition-colors hover:text-textPrimary"
                  aria-label={`Remove ${c.name || c.key}`}
                >
                  <X size={13} />
                </button>
              </div>
              {chatError && <p className="text-[12px] text-error">{chatError}</p>}
            </div>
          );
        })}
        {list.length < MAX_CHATS && <ChannelPicker label="Add a channel" onPick={addChannel} />}
      </div>
    </Field>
  );
}
