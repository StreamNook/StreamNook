// Opening a link that reached the app from outside: a channel, a badge or a
// category. Rust parses the URL and keeps the newest link in one slot
// (`services::inbound_link`); this file reads what Rust hands over and routes
// it, the same way for every shell.

/** What a link names, as Rust serializes it (`InboundLink`). */
export type InboundLinkTarget =
  | { kind: 'watch'; key: string }
  | { kind: 'badge'; setId: string; version: string }
  | { kind: 'category'; provider: 'twitch'; id: string };

/** Rust emits this with the stored link's stamp (a number) for every link. */
export const INBOUND_LINK_EVENT = 'streamnook:link';

/** What the category lookup answers (`CategoryAnswer` in Rust). */
export type CategoryAnswer =
  | { state: 'found'; id: string; name: string; box_art_url: string; viewer_count: number | null }
  | { state: 'missing' }
  | { state: 'unreachable' };

/** Read a link from Rust's JSON; null when it is not one. */
export function readInboundLink(raw: unknown): InboundLinkTarget | null {
  if (!raw || typeof raw !== 'object') return null;
  const link = raw as Record<string, unknown>;
  if (link.kind === 'watch') {
    return typeof link.key === 'string' && link.key ? { kind: 'watch', key: link.key } : null;
  }
  if (link.kind === 'badge') {
    if (typeof link.setId !== 'string' || !link.setId) return null;
    const version = typeof link.version === 'string' && link.version ? link.version : '1';
    return { kind: 'badge', setId: link.setId, version };
  }
  if (link.kind === 'category') {
    if (link.provider !== 'twitch') return null;
    if (typeof link.id !== 'string' || !/^\d{1,15}$/.test(link.id)) return null;
    return { kind: 'category', provider: 'twitch', id: link.id };
  }
  return null;
}

/** Read `take_pending_link`'s `{ link, at }`; `at` is ms on the device boot clock. */
export function readTakenLink(raw: unknown): { link: InboundLinkTarget; at: number } | null {
  if (!raw || typeof raw !== 'object') return null;
  const { link, at } = raw as { link?: unknown; at?: unknown };
  if (typeof at !== 'number' || !Number.isFinite(at)) return null;
  const read = readInboundLink(link);
  return read ? { link: read, at } : null;
}

/** Read the category lookup's answer; null when it is not one. */
export function readCategoryAnswer(raw: unknown): CategoryAnswer | null {
  if (!raw || typeof raw !== 'object') return null;
  const answer = raw as Record<string, unknown>;
  if (answer.state === 'missing') return { state: 'missing' };
  if (answer.state === 'unreachable') return { state: 'unreachable' };
  if (answer.state !== 'found') return null;
  if (typeof answer.id !== 'string' || !answer.id) return null;
  if (typeof answer.name !== 'string' || !answer.name) return null;
  return {
    state: 'found',
    id: answer.id,
    name: answer.name,
    box_art_url: typeof answer.box_art_url === 'string' ? answer.box_art_url : '',
    viewer_count:
      typeof answer.viewer_count === 'number' && Number.isFinite(answer.viewer_count)
        ? answer.viewer_count
        : null,
  };
}

/** What `openInboundLink` needs from the shell; stubbed in tests. */
export interface InboundLinkDeps {
  /** Starts the stream a watch link names. */
  startStream: (key: string) => void;
  /** Closes a badge sheet or placeholder, so it cannot stay over a new stream. */
  closeLinkSheet: () => void;
  /** Opens the badge a badge link names. */
  openBadge: (setId: string, version: string) => void;
  /** Closes the shell's own layers that sit over the screens. */
  closeAppLayers: () => void;
  /** Opens the category screen for an id, nameless; the screen names itself. */
  openCategory: (id: string) => void;
}

let held: { link: InboundLinkTarget; at: number } | null = null;

/** Keep one link for later (the phone's picture-in-picture hold). Last one wins. */
export function holdLink(link: InboundLinkTarget, at: number): void {
  held = { link, at };
}

/** Hand back the held link and forget it; null when none is held. */
export function takeHeld(): { link: InboundLinkTarget; at: number } | null {
  const out = held;
  held = null;
  return out;
}

/** Forget the held link. */
export function clearHeldLink(): void {
  held = null;
}

/**
 * Open a link. Synchronous for every kind: the screen the person sees next
 * frame is the one the link names, so nothing here waits for a network answer
 * (a category screen resolves its own nameless id).
 */
export function openInboundLink(link: InboundLinkTarget, deps: InboundLinkDeps): void {
  // A newer open replaces a link that was only waiting.
  clearHeldLink();
  switch (link.kind) {
    case 'watch':
      // An open badge sheet would otherwise stay over the stream.
      deps.closeLinkSheet();
      deps.startStream(link.key);
      return;
    case 'badge':
      deps.openBadge(link.setId, link.version);
      return;
    case 'category':
      deps.closeAppLayers();
      deps.openCategory(link.id);
      return;
  }
}

type LinkListen = (
  event: string,
  handler: (payload: unknown) => void,
) => Promise<() => void>;

/**
 * The one drain a shell runs for links. Tauri's `listen` is async, so a take on
 * mount can run before the listener exists and a link stored in that gap would
 * be emitted to nobody; the mount take therefore runs only after `listen`
 * settles (and still once when it is rejected, so a link stored before the
 * failed registration is not lost).
 */
export function startLinkDrain(deps: {
  listen: LinkListen;
  take: () => Promise<unknown>;
  /** `at` is the stored time; `mount` is true only for the take that follows `listen` settling. */
  open: (link: InboundLinkTarget, info: { at: number; mount: boolean }) => void;
  /** While it returns false nothing is taken: the link stays in Rust's slot. */
  ready?: () => boolean;
  /** Awaited before each take; a rejection is ignored. */
  beforeTake?: () => Promise<void>;
  /** Called with the stamp of every link event and every peek. */
  onStamp?: (stamp: number) => void;
  /** Reads the held link's stamp without taking it. */
  peek?: () => Promise<unknown>;
}): { drain: () => void; stop: () => void } {
  let cancelled = false;

  // Tauri hands a listener the whole event, the tests and the peek hand the number.
  const stamp = (value: unknown) => {
    const n =
      typeof value === 'object' && value !== null && 'payload' in value
        ? (value as { payload: unknown }).payload
        : value;
    if (typeof n === 'number' && n > 0) deps.onStamp?.(n);
  };

  const peekNow = () => {
    if (!deps.ready || deps.ready() || !deps.peek) return;
    try {
      void deps.peek().then(stamp, () => {});
    } catch {
      // A peek that cannot start is the same as no peek.
    }
  };

  const run = (mount: boolean) => {
    if (cancelled || (deps.ready && !deps.ready())) return;
    void (deps.beforeTake ? deps.beforeTake().catch(() => {}) : Promise.resolve())
      .then(() => (cancelled ? null : deps.take()))
      .then((raw) => {
        const taken = readTakenLink(raw);
        if (taken) deps.open(taken.link, { at: taken.at, mount });
      })
      .catch(() => {
        // An older backend without the command, or a link that failed to open.
      });
  };

  const onEvent = (payload: unknown) => {
    stamp(payload);
    run(false);
  };

  const postListen = () => {
    if (cancelled) return;
    peekNow();
    run(true);
  };

  peekNow();
  const registered = deps.listen(INBOUND_LINK_EVENT, onEvent);
  registered.then(postListen, postListen);

  return {
    drain: () => run(false),
    // A take already in flight still opens its link: the take emptied the slot.
    stop: () => {
      cancelled = true;
      void registered.then((unlisten) => unlisten(), () => {});
    },
  };
}

/**
 * The phone shell's link drain.
 * When this mount consumed a notification tap, the tap is the newer action, so
 * a watch link taken by the mount's own take is dropped; badge links still
 * open, and every later event opens whatever it carries.
 */
export function startPhoneLinkBridge(opts: {
  listen: LinkListen;
  take: () => Promise<unknown>;
  tapped: boolean;
  open: (link: InboundLinkTarget) => void;
}): { stop: () => void } {
  const drain = startLinkDrain({
    listen: opts.listen,
    take: opts.take,
    open: (link, { mount }) => {
      if (link.kind === 'watch' && opts.tapped && mount) return;
      opts.open(link);
    },
  });
  return { stop: drain.stop };
}
