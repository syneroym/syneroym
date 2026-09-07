/// Currencies with no minor unit at all.
export const EXPONENT_0 = new Set([
  "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG",
  "RWF", "UGX", "UYI", "VND", "VUV", "XAF", "XOF", "XPF",
]);

/// Currencies with three minor digits.
export const EXPONENT_3 = new Set(["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"]);

export class MoneyInputError extends Error {}
export const ListingInputError = MoneyInputError;

/// ISO-4217 minor-unit exponent. Two decimal places is the common case
/// and the default; only the currencies that are *not* two matter here,
/// because those are the ones a hard-coded "×100" mis-scales (a JPY price
/// by 100, a KWD price by one tenth). The two lists are the full set of
/// active exponent-0 and exponent-3 currencies, so any other code is two.
export function currencyMinorExponent(code: string): number {
  const c = code.trim().toUpperCase();
  if (EXPONENT_0.has(c)) return 0;
  if (EXPONENT_3.has(c)) return 3;
  return 2;
}

/// "35.50", exponent 2 -> 3550; "35", 2 -> 3500; "1200", 0 -> 1200;
/// "2.5", 3 -> 2500; "" -> undefined. Rejects an amount with more decimal
/// places than the currency has, so the result is always an integer
/// number of minor units.
export function toMinorUnits(input: string, exponent = 2): number | undefined {
  const t = input.trim();
  if (t === "") return undefined;
  const frac = exponent > 0 ? `(?:\\.(\\d{1,${exponent}}))?` : "";
  const m = new RegExp(`^(\\d+)${frac}$`).exec(t);
  if (!m) {
    const example = exponent > 0 ? `12 or 12.${"5".padEnd(exponent, "0")}` : "12";
    throw new MoneyInputError(
      `"${input}" is not an amount like ${example} for this currency`,
    );
  }
  const minor = m[1] + (m[2] ?? "").padEnd(exponent, "0");
  return Number.parseInt(minor, 10);
}

/// Formats integer minor units for display, e.g. 3550 USD -> "35.50 USD",
/// 1200 JPY -> "1200 JPY", 2500 KWD -> "2.500 KWD".
export function formatMinor(minor: number, currency: string): string {
  const c = currency.trim().toUpperCase();
  const exp = currencyMinorExponent(c);
  if (exp === 0) {
    return `${minor} ${c}`;
  }
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor).toString().padStart(exp + 1, "0");
  const whole = abs.slice(0, -exp);
  const frac = abs.slice(-exp);
  return `${sign}${whole}.${frac} ${c}`;
}
