// A notification's Rewards button asks for a channel's wallet to open. The
// chat for that channel may not be up yet (the click can start the stream), so
// the request waits here for a minute and the chat showing that channel takes
// it, once.

const WALLET_REQUEST_EVENT = 'sn-wallet-request';
const HOLD_MS = 60_000;

let pending: { login: string; at: number } | null = null;

export function requestWallet(login: string): void {
  pending = { login: login.toLowerCase(), at: Date.now() };
  window.dispatchEvent(new CustomEvent(WALLET_REQUEST_EVENT));
}

/** True once, for the chat showing `login`, while a request for it waits. */
export function takeWalletRequest(login: string | undefined): boolean {
  if (!pending || !login) return false;
  if (Date.now() - pending.at > HOLD_MS) {
    pending = null;
    return false;
  }
  if (pending.login !== login.toLowerCase()) return false;
  pending = null;
  return true;
}

/** Calls `fn` on each new request; returns the unsubscribe. */
export function onWalletRequest(fn: () => void): () => void {
  window.addEventListener(WALLET_REQUEST_EVENT, fn);
  return () => window.removeEventListener(WALLET_REQUEST_EVENT, fn);
}
