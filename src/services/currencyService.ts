// Currency conversion for YouTube Super Chats. YouTube formats amounts in the
// sender's local currency ("$5.00", "CA$5.00", "€5,00", "¥500"), so to show every
// Super Chat in one currency we parse the symbol, then convert via live rates.
//
// Rates: api.frankfurter.app (ECB data, free, no key), cached per-day in
// localStorage. Conversion is synchronous against the cached rates; a miss kicks a
// background fetch and returns null (caller shows the original until rates land).

import { invoke } from '@tauri-apps/api/core';
import { Logger } from '../utils/logger';

// YouTube Super Chat currency symbols -> ISO code. Longer prefixes first so "CA$"
// isn't mistaken for "$".
const SYMBOL_TO_CODE: Array<[string, string]> = [
  ['CA$', 'CAD'], ['AU$', 'AUD'], ['A$', 'AUD'], ['NZ$', 'NZD'], ['HK$', 'HKD'],
  ['NT$', 'TWD'], ['MX$', 'MXN'], ['US$', 'USD'], ['S$', 'SGD'], ['R$', 'BRL'],
  ['RM', 'MYR'], ['Rp', 'IDR'], ['CN¥', 'CNY'], ['JP¥', 'JPY'], ['zł', 'PLN'],
  ['CHF', 'CHF'], ['kr', 'SEK'], ['$', 'USD'], ['€', 'EUR'], ['£', 'GBP'],
  ['¥', 'JPY'], ['₩', 'KRW'], ['₹', 'INR'], ['₱', 'PHP'], ['₪', 'ILS'],
  ['₫', 'VND'], ['₺', 'TRY'], ['₴', 'UAH'], ['฿', 'THB'], ['₦', 'NGN'],
];

// The list lives with the overlay's pure currency module so the overlay builder
// can use it on the website too.
export { CURRENCY_OPTIONS } from '../components/overlay/currency';

const CODE_SYMBOL: Record<string, string> = {
  USD: '$', EUR: '€', GBP: '£', JPY: '¥', CAD: 'CA$', AUD: 'A$', NZD: 'NZ$',
  INR: '₹', BRL: 'R$', MXN: 'MX$', KRW: '₩', PHP: '₱', SGD: 'S$', HKD: 'HK$',
  TWD: 'NT$', CNY: '¥', THB: '฿', TRY: '₺', PLN: 'zł', IDR: 'Rp', MYR: 'RM',
};
const NO_DECIMAL = new Set(['JPY', 'KRW', 'VND', 'IDR', 'CLP', 'HUF']);

/** Map a YouTube currency symbol (or a bare ISO code) to an ISO code. */
export function symbolToCode(symbol: string | undefined): string | undefined {
  const s = (symbol ?? '').trim();
  if (!s) return undefined;
  for (const [sym, code] of SYMBOL_TO_CODE) if (s === sym) return code;
  return /^[A-Z]{3}$/.test(s) ? s : undefined;
}

let rates: Record<string, number> | null = null; // USD-based
let ratesDay = '';
let loading: Promise<void> | null = null;

function today(): string {
  return new Date().toISOString().slice(0, 10);
}

/** Ensure today's rates are loaded (localStorage cache, else fetch). Idempotent. */
export function preloadRates(): void {
  const day = today();
  if (rates && ratesDay === day) return;
  if (!rates) {
    try {
      const cached = JSON.parse(localStorage.getItem('sn_fx_rates') || 'null');
      if (cached?.day === day && cached.rates) {
        rates = cached.rates;
        ratesDay = day;
        return;
      }
    } catch {
      /* ignore */
    }
  }
  if (loading) return;
  loading = (async () => {
    try {
      // Fetched via the Rust backend: a browser fetch of frankfurter.app from the web
      // origin is CORS-blocked (no Access-Control-Allow-Origin). The API omits the
      // base currency from its rates, so seed USD = 1.
      const r = await invoke<Record<string, number>>('fetch_exchange_rates', { base: 'USD' });
      if (r && Object.keys(r).length > 0) {
        rates = { USD: 1, ...r };
        ratesDay = day;
        try {
          localStorage.setItem('sn_fx_rates', JSON.stringify({ day, rates }));
        } catch {
          /* ignore */
        }
      }
    } catch (e) {
      Logger.warn('[currency] rate fetch failed:', e);
    } finally {
      loading = null;
    }
  })();
}

/** Convert `amount` from one ISO code to another using cached rates. Returns null
 *  when rates aren't loaded yet or a currency is outside the rate set (caller then
 *  shows the original). Kicks a background load on a miss. */
export function convert(amount: number, fromCode: string, toCode: string): number | null {
  if (fromCode === toCode) return amount;
  if (!rates) {
    preloadRates();
    return null;
  }
  const rf = rates[fromCode];
  const rt = rates[toCode];
  if (!rf || !rt) return null;
  return (amount / rf) * rt;
}

/** Format an amount in an ISO currency (symbol + sensible decimals). */
export function formatMoney(amount: number, code: string): string {
  const sym = CODE_SYMBOL[code];
  const decimals = NO_DECIMAL.has(code) ? 0 : 2;
  const n = amount.toFixed(decimals);
  return sym ? `${sym}${n}` : `${n} ${code}`;
}
