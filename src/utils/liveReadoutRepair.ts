// The live readout shares its node with Plyr: Plyr writes its own session
// clock into `.plyr__time--current` on every timeupdate, and the player shows
// the broadcast timestamp there instead. A MutationObserver's callback runs as
// a microtask right after Plyr's write, in the same task and before the frame
// paints, so restoring our text there never lets Plyr's clock reach the
// screen, and nothing has to run every frame to catch it.

export interface ReadoutObserver {
  observe(target: Node, options: MutationObserverInit): void;
  disconnect(): void;
}

/** Builds the observer; injected so tests run without a DOM. */
export type ReadoutObserverFactory = (onChange: () => void) => ReadoutObserver;

export interface LiveReadoutRepairOptions {
  /** The player's container: watched whole, so a rebuilt control bar is seen. */
  root: Node;
  /** The readout node, looked up again whenever the cached one left the DOM. */
  find: () => HTMLElement | null;
  /** What the node must show, or null to leave it alone (at the live edge the
   *  node is hidden and the LIVE button carries the state). */
  text: () => string | null;
  observe?: ReadoutObserverFactory;
}

const domObserver: ReadoutObserverFactory = (onChange) => new MutationObserver(onChange);

/** Keep the readout showing `text()`. Returns the stop. Our own write is
 *  observed too and finds the text already right, so it never loops. */
export function liveReadoutRepair({ root, find, text, observe = domObserver }: LiveReadoutRepairOptions): () => void {
  let node: HTMLElement | null = null;
  const repair = () => {
    if (!node || !node.isConnected) node = find();
    const want = text();
    if (!node || want === null) return;
    if (node.textContent !== want) node.textContent = want;
  };
  const observer = observe(repair);
  observer.observe(root, { childList: true, characterData: true, subtree: true });
  return () => observer.disconnect();
}
