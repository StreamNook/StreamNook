// The App-local layers a category link closes before its screen opens. Built
// once from stable parts (a ref and state setters), so the link drain can
// capture the result without it going stale, and a test can drive it without
// mounting App.

export function makeCloseAppLayers({
  changelogRecordsSeenRef,
  setShowChangelog,
  setSelectedBadge,
}: {
  changelogRecordsSeenRef: { current: boolean };
  setShowChangelog: (open: boolean) => void;
  setSelectedBadge: (next: null) => void;
}): () => void {
  return () => {
    // A changelog that opened after an update is kept: closing it through this
    // path would skip the step that records the version as seen.
    if (!changelogRecordsSeenRef.current) setShowChangelog(false);
    // Clears the badge detail and its link placeholder alike.
    setSelectedBadge(null);
  };
}
