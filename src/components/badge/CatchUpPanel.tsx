import { usdLabel, watchTimeLabel, type CatchUp } from '../../services/badgeStanding';

/** What earning everything above takes, in money and in watching. The math
 *  (effort shared within a category, US Tier 1 price) is Rust's `CatchUp`. */
export const CatchUpPanel = ({ totals, stacked = false }: { totals: CatchUp; stacked?: boolean }) => {
  const stats: { value: string; label: string }[] = [];
  if (totals.subs > 0) {
    stats.push({ value: usdLabel(totals.sub_cost_cents), label: totals.subs === 1 ? 'in 1 sub' : `in ${totals.subs} subs` });
  }
  if (totals.watch_minutes > 0) stats.push({ value: watchTimeLabel(totals.watch_minutes), label: 'of watching' });
  if (totals.tickets > 0) {
    stats.push({ value: String(totals.tickets), label: totals.tickets === 1 ? 'event pass' : 'event passes' });
  }
  if (stats.length === 0) return null;

  const notes = [
    'Subs and watch time in the same category count toward every badge there, so they are counted once. Subs at the US price of $5.99.',
    totals.random > 0 &&
      `${totals.random === 1 ? 'One badge is' : `${totals.random} badges are`} drawn at random, so it may take more.`,
    totals.unpriced > 0 &&
      `${totals.unpriced === 1 ? 'One badge has' : `${totals.unpriced} badges have`} no cost to add up (Bits, creator tasks).`,
    totals.estimated && 'Some numbers come from badge descriptions rather than Twitch.',
  ].filter(Boolean);
  const floor = totals.random > 0 || totals.unpriced > 0;

  return (
    <div
      className={`missing-now-catchup glaze-inset${stacked ? ' missing-now-catchup--stacked' : ''}`}
      title={notes.join('\n')}
    >
      <span className="missing-now-catchup-label">{floor ? 'To catch up, at least' : 'To catch up'}</span>
      <div className="missing-now-stats">
        {stats.map((s) => (
          <div key={s.label} className="missing-now-stat">
            <span className="missing-now-stat-value">{s.value}</span>
            <span className="missing-now-stat-label">{s.label}</span>
          </div>
        ))}
      </div>
    </div>
  );
};
