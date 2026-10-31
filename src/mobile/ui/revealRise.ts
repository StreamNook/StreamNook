// Whether a kept surface (Chats) rises into place when it shows. Shown while
// the full player covered the shell (Close video lands on Chats that way), it
// is already in place when the player fades, so it does not rise as well.
//
// "Covered" is the value of the render before: the player letting go and the
// surface showing are two store writes in one run, which React commits in one
// render, so the render that shows the surface already reads "not covered".
export function createRevealRise(shown: boolean, covered: boolean): (shown: boolean, covered: boolean) => boolean {
  let prevShown = shown;
  let prevCovered = covered;
  return (nowShown, nowCovered) => {
    const rise = nowShown && !prevShown && !prevCovered;
    prevShown = nowShown;
    prevCovered = nowCovered;
    return rise;
  };
}
