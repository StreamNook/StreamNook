import { useEffect, useState } from 'react';
import { Crown } from 'lucide-react';
import {
  getStreamNookMembership,
  type StreamNookMembership,
} from '../../services/supabaseService';
import {
  getMembershipStatus,
  membershipStatusCopy,
  onMembershipStatusChanged,
  type MembershipStatus,
  type MembershipStatusCopy,
} from '../../services/membershipStatus';

// The member's own StreamNook membership: a status line (paid or gifted, when
// a gift ends and when their own billing resumes, as streamnook.app computes
// it) plus lifetime tenure. Renders nothing for members with neither.

const dateLabel = (iso: string): string => new Date(iso).toLocaleDateString();

const daysSince = (iso: string): number =>
  Math.max(0, Math.floor((Date.now() - new Date(iso).getTime()) / 86_400_000));

const STATUS_PILL: Record<string, { label: string; className: string }> = {
  active: { label: 'Active', className: 'bg-accent/20 text-accent' },
  trialing: { label: 'Active', className: 'bg-accent/20 text-accent' },
  ending: { label: 'Ending', className: 'bg-amber-400/10 text-amber-300' },
  past_due: { label: 'Past due', className: 'bg-amber-400/10 text-amber-300' },
  canceled: { label: 'Canceled', className: 'bg-white/[0.06] text-textSecondary' },
};

const TONE_DOT: Record<MembershipStatusCopy['tone'], string> = {
  gift: 'bg-accent',
  ok: 'bg-emerald-400',
  warn: 'bg-amber-300',
};

const Row = ({ label, value }: { label: string; value: string }) => (
  <div className="flex items-center justify-between gap-3 rounded-md px-2 py-1.5 hover:bg-white/[0.03]">
    <span className="text-sm text-textSecondary">{label}</span>
    <span className="text-[13px] tabular-nums text-textPrimary">{value}</span>
  </div>
);

/** A status is a fact, not a control: a dot and words. */
const StatusLine = ({ copy }: { copy: MembershipStatusCopy }) => (
  <div className="mb-3 px-2">
    <div className="flex items-start gap-2">
      <span aria-hidden className={`mt-[7px] h-1.5 w-1.5 shrink-0 rounded-full ${TONE_DOT[copy.tone]}`} />
      <span className="text-sm font-medium text-textPrimary">{copy.headline}</span>
    </div>
    {copy.detail && (
      <p className="mt-0.5 pl-3.5 text-[12px] leading-snug text-textSecondary">{copy.detail}</p>
    )}
  </div>
);

const MembershipSection = ({ userId }: { userId: string }) => {
  const [membership, setMembership] = useState<StreamNookMembership | null>(null);
  const [status, setStatus] = useState<MembershipStatus | null>(null);

  useEffect(() => {
    let alive = true;
    getStreamNookMembership(userId).then((m) => {
      if (alive) setMembership(m);
    });
    return () => { alive = false; };
  }, [userId]);

  // Rust fetches and caches the status; a gift arriving while the profile is
  // open pushes the new one.
  useEffect(() => {
    let alive = true;
    getMembershipStatus().then((s) => {
      if (alive) setStatus(s);
    });
    const unlisten = onMembershipStatusChanged((s) => {
      if (alive) setStatus(s);
    });
    return () => {
      alive = false;
      unlisten.then((fn) => fn()).catch(() => {});
    };
  }, [userId]);

  const copy = status ? membershipStatusCopy(status) : null;
  if (!membership && !copy) return null;
  const m = membership;

  const live = m ? m.status === 'active' || m.status === 'past_due' || m.status === 'trialing' : false;
  const pillKey = m && live && m.cancelAtPeriodEnd && m.status !== 'past_due' ? 'ending' : (m?.status ?? '');
  // The status line replaces the pill and the renewal row when the server
  // answered; both stay as the fallback for an older server or offline.
  const pill = copy ? null : STATUS_PILL[pillKey] ?? null;

  return (
    <div className="glass-panel rounded-xl p-5">
      <div className="mb-4 flex items-center gap-1.5">
        <Crown size={14} className="text-textMuted" />
        <h4 className="text-sm font-semibold uppercase tracking-wide text-textPrimary">
          StreamNook Membership
        </h4>
        {pill && (
          <span className={`ml-auto rounded px-1.5 py-0.5 text-[10px] font-semibold ${pill.className}`}>
            {pill.label}
          </span>
        )}
      </div>

      {copy && <StatusLine copy={copy} />}

      {m && (
        <div className="space-y-1">
          {m.firstSubscribedAt && (
            <Row
              label="Member since"
              value={`${dateLabel(m.firstSubscribedAt)} · ${daysSince(m.firstSubscribedAt).toLocaleString()} days`}
            />
          )}
          <Row label="Months supported" value={String(m.totalMonths)} />
          {!copy && live && m.currentPeriodEnd && (
            <Row
              label={m.cancelAtPeriodEnd ? 'Ends on' : 'Renews on'}
              value={dateLabel(m.currentPeriodEnd)}
            />
          )}
          {!live && m.status === 'canceled' && (m.endedAt || m.currentPeriodEnd) && (
            <Row label="Ended on" value={dateLabel((m.endedAt ?? m.currentPeriodEnd)!)} />
          )}
          {m.canceledAt && (
            <Row label="Canceled on" value={dateLabel(m.canceledAt)} />
          )}
          {m.lastPaidAt && <Row label="Last payment" value={dateLabel(m.lastPaidAt)} />}
        </div>
      )}

      {!copy && m && live && m.cancelAtPeriodEnd && m.currentPeriodEnd && (
        <p className="mt-3 text-[11px] leading-snug text-textSecondary">
          Your subscription stays active through {dateLabel(m.currentPeriodEnd)}. Earned badges and
          unlocks are yours to keep either way.
        </p>
      )}
    </div>
  );
};

export default MembershipSection;
