import { ChevronLeft, ChevronRight } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';
import { useAppStore } from '../stores/AppStore';
import { Tooltip } from './ui/Tooltip';

/** The app's one navigation control: back and forward, in the title bar,
 *  wherever you happen to be.
 *
 *  It replaces a single button that swapped its own icon with the state — a
 *  house when you could leave, a TV when you could come back — and a separate
 *  floating back arrow that lived over the category hero. Both worked, and both
 *  had to be READ before they could be used: the same pixels meant different
 *  things depending on where you already were, and the second one moved
 *  depending on which view you were in. A flipper's halves never change
 *  meaning, and this one never changes place.
 *
 *  The path it walks, outermost first:
 *
 *      browse  <->  category  <->  (Home)  <->  stream
 *
 *  Back always steps out, forward always steps back in, and the two retrace
 *  the same route. The rules live in `navigateBack` / `navigateForward` on the
 *  store rather than here, because Home owns the category and the player owns
 *  the stream, and a control in the title bar can see neither.
 *
 *  The unavailable half stays in place rather than disappearing, so the control
 *  never changes width and the live half never slides out from under the
 *  pointer between clicks. */
const NavFlipper = () => {
  // Actions are stable for the store's lifetime, so take them without
  // subscribing; state goes through a shallow-compared selector.
  const { navigateBack, navigateForward } = useAppStore.getState();
  const { isHomeActive, streamUrl, homeActiveTab, selectedCategory, lastExitedCategory } = useAppStore(
    useShallow((s) => ({
      isHomeActive: s.isHomeActive,
      streamUrl: s.streamUrl,
      homeActiveTab: s.homeActiveTab,
      selectedCategory: s.homeSelectedCategory,
      lastExitedCategory: s.homeLastExitedCategory,
    })),
  );

  const watching = !isHomeActive && !!streamUrl;
  const inCategory = homeActiveTab === 'category' && !!selectedCategory;
  const canBack = watching || inCategory;
  const canForward =
    isHomeActive && ((homeActiveTab === 'browse' && !!lastExitedCategory) || !!streamUrl);

  // Nowhere to go in either direction means the control has no job. It is the
  // one moment it should be absent rather than merely dim: two dead halves
  // read as broken, where no control at all reads as "you are at the start".
  if (!canBack && !canForward) return null;

  const backLabel = watching ? 'Keep browsing' : 'Back to browse';
  const forwardLabel =
    homeActiveTab === 'browse' && lastExitedCategory
      ? `Back to ${lastExitedCategory.name}`
      : 'Return to stream';

  return (
    <div className="chrome-glaze nav-flipper">
      <Tooltip content={backLabel} delay={200}>
        <button
          type="button"
          className="nav-flipper-btn"
          disabled={!canBack}
          aria-label={backLabel}
          onClick={() => navigateBack()}
        >
          <ChevronLeft size={20} />
        </button>
      </Tooltip>

      <span className="nav-flipper-divider" aria-hidden="true" />

      <Tooltip content={forwardLabel} delay={200}>
        <button
          type="button"
          className="nav-flipper-btn"
          disabled={!canForward}
          aria-label={forwardLabel}
          onClick={() => navigateForward()}
        >
          <ChevronRight size={20} />
        </button>
      </Tooltip>
    </div>
  );
};

export default NavFlipper;
