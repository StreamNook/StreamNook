// A chat tab's name with its live dot. The dot hangs off the name's top-right
// corner inside the tab's padding, so a channel going live or offline never
// moves the name or widens the tab. MultiChat's tabs and the MultiNook chat
// switcher both draw it, so the two read the same.
export function TabLiveLabel({ label, isLive, className = '' }: { label: string; isLive: boolean; className?: string }) {
  return (
    <span className="relative min-w-0">
      <span className={`block truncate ${className}`}>{label}</span>
      {isLive && <span className="blend-live-dot pointer-events-none absolute -right-1.5 -top-0.5" aria-hidden />}
    </span>
  );
}
