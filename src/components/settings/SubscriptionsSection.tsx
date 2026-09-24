import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { motion } from 'framer-motion';
import { DollarSign, Gift, Crown, Star, ChevronRight } from 'lucide-react';
import { Tooltip } from '../ui/Tooltip';
import { Logger } from '../../utils/logger';
import {
  fetchMySubscriptions,
  fetchMyPastSubscriptions,
  type MySubscription,
  type PastSubscription,
} from '../../services/twitchService';
import type { IvrSubageSummary } from '../../types';

// USD list price per tier per month. The spend figure is an estimate only:
// regional pricing varies, active Prime / gifted subs cost nothing (and are
// excluded), and past subs don't expose Prime/gift status so they are counted
// at tier price.
const TIER_PRICE: Record<1 | 2 | 3, number> = { 1: 4.99, 2: 9.99, 3: 24.99 };

const containerV = { hidden: {}, show: { transition: { staggerChildren: 0.02 } } };
const itemV = {
  hidden: { opacity: 0, y: 6 },
  show: { opacity: 1, y: 0, transition: { duration: 0.2, ease: 'easeOut' as const } },
};

const activeTierLabel = (s: MySubscription): string =>
  s.isGift ? 'Gift' : s.isPrime ? 'Prime' : `Tier ${s.tier}`;

interface Row {
  key: string;
  name: string;
  label: string;
  months: number | undefined; // undefined while the tenure lookup is still loading
  status: 'active' | 'past';
  free: boolean; // active Prime/gift -> muted pill
}

const SubscriptionsSection = ({ login }: { login: string }) => {
  // null = still loading active list; [] = loaded but none (or fetch failed).
  const [subs, setSubs] = useState<MySubscription[] | null>(null);
  const [past, setPast] = useState<PastSubscription[]>([]);
  // Total cumulative months subscribed per ACTIVE channel, from IVR (lifetime
  // tenure, not the current unbroken streak). Absent key = still loading.
  const [cumulativeMonths, setCumulativeMonths] = useState<Record<string, number>>({});
  const [tenureLoaded, setTenureLoaded] = useState(false);
  const [expanded, setExpanded] = useState(false);

  const self = login.toLowerCase();

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const [active, pastList] = await Promise.all([
          fetchMySubscriptions(),
          fetchMyPastSubscriptions(),
        ]);
        if (!alive) return;
        setSubs(active);
        setPast(pastList);

        // Total cumulative months for each active sub (skip our own channel).
        // IVR is only hit for the handful of active subs, so no large fan-out /
        // rate-limit risk.
        const entries = await Promise.all(
          active
            .filter((s) => s.channelLogin.toLowerCase() !== self)
            .map(async (s) => {
              const data = await invoke<IvrSubageSummary | null>('get_ivr_subage_summary', {
                login,
                channel: s.channelLogin,
              }).catch(() => null);
              // Lifetime months subscribed, not the current streak.
              return [s.channelLogin.toLowerCase(), data?.lifetime_months ?? 0] as const;
            }),
        );
        if (alive) {
          setCumulativeMonths(Object.fromEntries(entries));
          setTenureLoaded(true);
        }
      } catch (e) {
        Logger.error('[Subscriptions] Failed to load:', e);
        if (alive) {
          setSubs([]);
          setTenureLoaded(true);
        }
      }
    })();
    return () => { alive = false; };
  }, [login, self]);

  const loading = subs === null;

  // Exclude our own channel (broadcasters get a default Tier 3 to themselves).
  const active = (subs ?? []).filter((s) => s.channelLogin.toLowerCase() !== self);
  const pastAll = past.filter((p) => p.channelLogin.toLowerCase() !== self);

  const paid = active.filter((s) => !s.isPrime && !s.isGift);
  const primeCount = active.filter((s) => s.isPrime).length;
  const giftCount = active.filter((s) => s.isGift && !s.isPrime).length;

  // Sum past periods per channel (a channel can have several lapsed periods).
  const pastByChannel = new Map<string, { name: string; tier: 1 | 2 | 3; months: number }>();
  for (const p of pastAll) {
    const k = p.channelLogin.toLowerCase();
    const cur = pastByChannel.get(k);
    if (cur) {
      cur.months += p.months;
      cur.tier = Math.max(cur.tier, p.tier) as 1 | 2 | 3;
    } else {
      pastByChannel.set(k, { name: p.channelDisplayName, tier: p.tier, months: p.months });
    }
  }
  const activeByChannel = new Map(active.map((s) => [s.channelLogin.toLowerCase(), s]));

  // ── Spend ──────────────────────────────────────────────────────────────
  // Count each channel's FULL tenure once. Active channels use IVR cumulative
  // months (the lifetime total, which already includes any earlier lapsed
  // periods) at the current tier. Past-only channels price each lapsed period
  // at its own tier. Cumulative already covers a channel's past periods, so
  // those past entries are skipped for channels that also have an active sub,
  // and nothing is double-counted.
  let spend = 0;
  let totalMonths = 0;
  for (const s of active) {
    const m = cumulativeMonths[s.channelLogin.toLowerCase()] || 0;
    totalMonths += m;
    if (!s.isPrime && !s.isGift) spend += TIER_PRICE[s.tier] * Math.max(1, m);
  }
  for (const p of pastAll) {
    if (activeByChannel.has(p.channelLogin.toLowerCase())) continue; // covered by cumulative
    spend += TIER_PRICE[p.tier] * p.months;
    totalMonths += p.months;
  }

  // ── Display rows: union of active + past channels ──────────────────────
  const rowKeys = new Set<string>([...activeByChannel.keys(), ...pastByChannel.keys()]);
  const rows: Row[] = [...rowKeys]
    .map((k): Row => {
      const a = activeByChannel.get(k);
      const pst = pastByChannel.get(k);
      const tenureKnown = !a || tenureLoaded;
      // Active: cumulative tenure (already includes any past periods for this
      // channel). Past-only: the summed lapsed months.
      const months = a ? cumulativeMonths[k] || 0 : pst!.months;
      return {
        key: k,
        name: a?.channelDisplayName ?? pst!.name,
        label: a ? activeTierLabel(a) : `Tier ${pst!.tier}`,
        months: tenureKnown ? months : undefined,
        status: a ? 'active' : 'past',
        free: a ? a.isPrime || a.isGift : false,
      };
    })
    .sort((x, y) => (y.months || 0) - (x.months || 0));

  return (
    <div className="glass-panel rounded-xl p-5">
      <div className="mb-4 flex items-center gap-1.5">
        <Star size={14} className="text-textMuted" />
        <h4 className="text-sm font-semibold uppercase tracking-wide text-textPrimary">
          Subscriptions
        </h4>
        {!loading && (
          <span className="ml-auto text-[11px] tabular-nums text-textMuted">
            {rows.length} channels
          </span>
        )}
      </div>

      {loading ? (
        <p className="text-sm italic text-textSecondary">Loading your subscriptions…</p>
      ) : rows.length === 0 ? (
        <p className="text-sm italic text-textSecondary">No subscriptions found.</p>
      ) : (
        <>
          {/* Spend headline */}
          <div className="glass-tile mb-4 p-4">
            <div className="mb-1 flex items-center gap-2">
              <DollarSign size={17} strokeWidth={2.25} style={{ color: '#8fd4b4' }} />
              <span className="text-[11px] font-medium uppercase tracking-[0.1em] text-textMuted">
                Poured into Twitch
              </span>
            </div>
            <div className="flex items-baseline gap-1.5">
              <span className="text-3xl font-bold tabular-nums text-textPrimary">
                ${Math.round(spend).toLocaleString()}
              </span>
              <span className="text-xs text-textMuted">
                estimated across {totalMonths.toLocaleString()} months
              </span>
            </div>
            <p className="mt-1.5 text-[11px] leading-snug text-textSecondary">
              Tier price times your total months subscribed to each channel. Active Prime and gifts
              are excluded; past subs are counted at tier price (their Prime/gift status isn't
              exposed). US pricing assumed, so treat it as a ballpark.
            </p>
          </div>

          {/* Breakdown */}
          <div className="mb-4 flex flex-wrap gap-2 text-[11px]">
            <span className="rounded-full border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-textSecondary">
              {paid.length} paid
            </span>
            {primeCount > 0 && (
              <span className="flex items-center gap-1 rounded-full border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-textSecondary">
                <Crown size={11} /> {primeCount} Prime
              </span>
            )}
            {giftCount > 0 && (
              <span className="flex items-center gap-1 rounded-full border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-textSecondary">
                <Gift size={11} /> {giftCount} gifted
              </span>
            )}
            {pastByChannel.size > 0 && (
              <span className="rounded-full border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-textMuted">
                {pastByChannel.size} past
              </span>
            )}
          </div>

          {/* Channels - collapsed by default; expand to see the full list */}
          <button
            type="button"
            onClick={() => setExpanded((v) => !v)}
            className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-left text-[13px] font-medium text-textSecondary transition-colors hover:bg-white/[0.03] hover:text-textPrimary"
          >
            <ChevronRight
              size={14}
              className={`transition-transform duration-200 ${expanded ? 'rotate-90' : ''}`}
            />
            {expanded ? 'Hide channels' : `Show all ${rows.length} channels`}
          </button>
          {expanded && (
            <motion.div
              variants={containerV}
              initial="hidden"
              animate="show"
              className="mt-1 space-y-1"
            >
              {rows.map((r) => (
                <motion.div
                  key={r.key}
                  variants={itemV}
                  className="flex items-center gap-2 rounded-md px-2 py-1.5 hover:bg-white/[0.03]"
                >
                  <span
                    className={`min-w-0 flex-1 truncate text-sm ${
                      r.status === 'past' ? 'text-textSecondary' : 'text-textPrimary'
                    }`}
                  >
                    {r.name}
                  </span>
                  <Tooltip content={r.status === 'past' ? `${r.label} (ended)` : r.label} side="left">
                    <span
                      className={`rounded px-1.5 py-0.5 text-[10px] font-semibold ${
                        r.status === 'active' && !r.free
                          ? 'bg-accent/20 text-accent'
                          : 'bg-white/[0.06] text-textSecondary'
                      }`}
                    >
                      {r.label}
                    </span>
                  </Tooltip>
                  <span className="w-16 text-right text-[11px] tabular-nums text-textMuted">
                    {r.months === undefined ? '…' : r.months > 0 ? `${r.months} mo` : ''}
                  </span>
                </motion.div>
              ))}
            </motion.div>
          )}
        </>
      )}
    </div>
  );
};

export default SubscriptionsSection;
