import { create } from 'zustand';
import { formatBits, sendCheer, type CheerCheck } from '../services/bitsService';
import { Logger } from '../utils/logger';

/**
 * A cheer on its way out, for the cheer bar above the message box.
 *
 * While the viewer types, the bar shows Rust's live check of the box
 * (`drafts`, per channel): what it will spend and anything in the way. Send
 * then goes through Rust as usual; Rust holds any message that cheers and
 * hands it back (`hold`). When the bar already showed that exact message with
 * nothing in the way, the hold is the viewer's yes and the cheer goes out at
 * once. Otherwise the bar asks once (`held`), or says what stopped it
 * (`refused`) and puts the message back in the box (`restore`).
 */
export type CheerStatus =
  | { kind: 'held'; check: CheerCheck }
  | { kind: 'sending'; check: CheerCheck }
  | { kind: 'refused'; check: CheerCheck; problem: string; canContinue: boolean; needsBits: boolean };

interface CheerState {
  drafts: Record<string, CheerCheck>;
  status: CheerStatus | null;
  /** Text to put back in the box of `channel` (bumped per restore). */
  restore: { channel: string; text: string; seq: number } | null;
  setDraft: (channel: string, check: CheerCheck | null) => void;
  hold: (check: CheerCheck) => void;
  send: (check: CheerCheck, continueAnyway?: boolean) => Promise<void>;
  dismiss: () => void;
}

let restoreSeq = 0;

export const useCheerPromptStore = create<CheerState>((set, get) => ({
  drafts: {},
  status: null,
  restore: null,
  setDraft: (channel, check) =>
    set((s) => {
      if (!check && !(channel in s.drafts)) return s;
      const drafts = { ...s.drafts };
      if (check) drafts[channel] = check;
      else delete drafts[channel];
      return { drafts };
    }),
  hold: (check) => {
    const seen = get().drafts[check.channel];
    if (check.problems.length > 0) {
      set({
        status: { kind: 'refused', check, problem: check.problems[0], canContinue: false, needsBits: check.needs_bits },
        restore: { channel: check.channel, text: check.text, seq: ++restoreSeq },
      });
      return;
    }
    // The bar showed this very message, cheer and all: Send was the yes.
    if (seen && seen.text.trim() === check.text.trim() && seen.problems.length === 0) {
      void get().send(check);
      return;
    }
    set({ status: { kind: 'held', check } });
  },
  send: async (check, continueAnyway = false) => {
    set({ status: { kind: 'sending', check } });
    try {
      const outcome = await sendCheer(check.channel, check.channel_id, check.text, continueAnyway);
      if (outcome.sent) {
        set((s) => {
          const drafts = { ...s.drafts };
          delete drafts[check.channel];
          return { status: null, drafts };
        });
        return;
      }
      set({
        status: {
          kind: 'refused',
          check: outcome.balance !== null ? { ...check, balance: outcome.balance } : check,
          problem: outcome.problem ?? `Your cheer of ${formatBits(check.total)} bits didn't go out.`,
          canContinue: outcome.can_continue,
          needsBits: outcome.needs_bits,
        },
        restore: { channel: check.channel, text: check.text, seq: ++restoreSeq },
      });
    } catch (err) {
      Logger.warn('[Cheer] send failed:', err);
      set({
        status: {
          kind: 'refused',
          check,
          problem: typeof err === 'string' ? err : "Your cheer couldn't be sent. Nothing was spent.",
          canContinue: false,
          needsBits: false,
        },
        restore: { channel: check.channel, text: check.text, seq: ++restoreSeq },
      });
    }
  },
  dismiss: () => set({ status: null }),
}));

/** The live check of `text` in `channel`'s box, when it cheers. */
export function useCheerDraft(channel: string | null | undefined, text: string | undefined): CheerCheck | null {
  const key = channel ? channel.toLowerCase().replace(/^#/, '') : '';
  const draft = useCheerPromptStore((s) => (key ? s.drafts[key] : undefined));
  return draft && text !== undefined && draft.text === text ? draft : null;
}
