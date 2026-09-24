// Portable Super Chat currency conversion for the overlay renderer. Kept dependency-
// free (no Tauri, no app services) so it runs identically in the in-app builder
// preview and on the hosted overlay page. Rates come from streamnook.app/api/fx-rates
// (a Functions proxy of frankfurter.app), cached in memory + per-day localStorage.

const RATES_URL = 'https://streamnook.app/api/fx-rates';

// YouTube Super Chat currency symbols → ISO code. Longer prefixes first so "CA$"
// isn't mistaken for "$".
const SYMBOL_TO_CODE: Array<[string, string]> = [
  ['CA$', 'CAD'], ['AU$', 'AUD'], ['A$', 'AUD'], ['NZ$', 'NZD'], ['HK$', 'HKD'],
  ['NT$', 'TWD'], ['MX$', 'MXN'], ['US$', 'USD'], ['S$', 'SGD'], ['R$', 'BRL'],
  ['RM', 'MYR'], ['Rp', 'IDR'], ['CN¥', 'CNY'], ['JP¥', 'JPY'], ['zł', 'PLN'],
  ['CHF', 'CHF'], ['kr', 'SEK'], ['$', 'USD'], ['€', 'EUR'], ['£', 'GBP'],
  ['¥', 'JPY'], ['₩', 'KRW'], ['₹', 'INR'], ['₱', 'PHP'], ['₪', 'ILS'],
  ['₫', 'VND'], ['₺', 'TRY'], ['₴', 'UAH'], ['฿', 'THB'], ['₦', 'NGN'],
];

const CODE_SYMBOL: Record<string, string> = {
  USD: '$', EUR: '€', GBP: '£', JPY: '¥', CAD: 'CA$', AUD: 'A$', NZD: 'NZ$',
  INR: '₹', BRL: 'R$', MXN: 'MX$', KRW: '₩', PHP: '₱', SGD: 'S$', HKD: 'HK$',
  TWD: 'NT$', CNY: '¥', THB: '฿', TRY: '₺', PLN: 'zł', IDR: 'Rp', MYR: 'RM',
};
const NO_DECIMAL = new Set(['JPY', 'KRW', 'VND', 'IDR', 'CLP', 'HUF']);

// A money token in a system message: a currency symbol/prefix then a number.
const MONEY_RE =
  /(CA\$|AU\$|A\$|NZ\$|HK\$|NT\$|MX\$|US\$|S\$|R\$|CN¥|JP¥|zł|CHF|RM|Rp|kr|[$€£¥₩₹₱₪₫₺₴฿₦])\s?(\d[\d.,]*)/;

let rates: Record<string, number> | null = null;
let ratesDay = '';
let loading: Promise<void> | null = null;

const today = () => new Date().toISOString().slice(0, 10);

/** True once today's rates are in memory. */
// Targets a user can pick (all present in the Frankfurter set).
export const CURRENCY_OPTIONS = [
  'USD', 'EUR', 'GBP', 'CAD', 'AUD', 'JPY', 'INR', 'BRL', 'MXN', 'KRW', 'PHP',
  'SGD', 'SEK', 'CHF', 'PLN', 'TRY', 'ZAR', 'NZD', 'HKD', 'TWD', 'THB', 'IDR', 'MYR', 'CNY',
];

export function ratesReady(): boolean {
  return !!rates && ratesDay === today();
}

/** Ensure today's rates are loaded (localStorage cache, else fetch the proxy).
 *  Resolves when rates are available (or the fetch fails). Idempotent. */
export function loadRates(): Promise<void> {
  const day = today();
  if (rates && ratesDay === day) return Promise.resolve();
  if (typeof localStorage !== 'undefined') {
    try {
      const cached = JSON.parse(localStorage.getItem('sn_overlay_fx') || 'null');
      if (cached?.day === day && cached.rates) {
        rates = cached.rates;
        ratesDay = day;
        return Promise.resolve();
      }
    } catch {
      /* ignore */
    }
  }
  if (loading) return loading;
  loading = (async () => {
    try {
      const res = await fetch(RATES_URL);
      if (res.ok) {
        const data = (await res.json()) as { rates?: Record<string, number> };
        if (data.rates && Object.keys(data.rates).length > 0) {
          rates = data.rates;
          ratesDay = day;
          if (typeof localStorage !== 'undefined') {
            try {
              localStorage.setItem('sn_overlay_fx', JSON.stringify({ day, rates }));
            } catch {
              /* ignore */
            }
          }
        }
      }
    } catch {
      /* leave rates null; caller shows the original amount */
    } finally {
      loading = null;
    }
  })();
  return loading;
}

function symbolToCode(symbol: string): string | undefined {
  const s = symbol.trim();
  for (const [sym, code] of SYMBOL_TO_CODE) if (s === sym) return code;
  return /^[A-Z]{3}$/.test(s) ? s : undefined;
}

function parseAmount(raw: string): number | null {
  // If it has a '.', commas are thousands separators; else a comma is the decimal.
  const normalized = raw.includes('.') ? raw.replace(/,/g, '') : raw.replace(/,/g, '.');
  const n = parseFloat(normalized);
  return Number.isFinite(n) ? n : null;
}

function formatMoney(amount: number, code: string): string {
  const sym = CODE_SYMBOL[code];
  const decimals = NO_DECIMAL.has(code) ? 0 : 2;
  const n = amount.toFixed(decimals);
  return sym ? `${sym}${n}` : `${n} ${code}`;
}

/** Replace the first money token in `text` with its value in `targetCode`. Returns
 *  the text unchanged if rates aren't loaded, the currency is unknown, or there's no
 *  money token — so a caller can always show the result. */
export function convertMoneyInText(text: string, targetCode: string): string {
  if (!targetCode || !rates) return text;
  const m = text.match(MONEY_RE);
  if (!m) return text;
  const from = symbolToCode(m[1]);
  const amount = parseAmount(m[2]);
  if (!from || amount == null) return text;
  if (from === targetCode) return text;
  const rf = rates[from];
  const rt = rates[targetCode];
  if (!rf || !rt) return text;
  const converted = (amount / rf) * rt;
  return text.replace(m[0], formatMoney(converted, targetCode));
}
