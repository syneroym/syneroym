/// Currencies with no minor unit at all.
export const EXPONENT_0 = new Set([
  "BIF", "CLP", "DJF", "GNF", "ISK", "JPY", "KMF", "KRW", "PYG",
  "RWF", "UGX", "UYI", "VND", "VUV", "XAF", "XOF", "XPF",
]);

/// Currencies with three minor digits.
export const EXPONENT_3 = new Set(["BHD", "IQD", "JOD", "KWD", "LYD", "OMR", "TND"]);

/// Every ISO-4217 alphabetic code this build accepts, sorted.
export const CURRENCY_CODES = new Set([
  "AED", "AFN", "ALL", "AMD", "ANG", "AOA", "ARS", "AUD", "AWG", "AZN", "BAM", "BBD", "BDT",
  "BGN", "BHD", "BIF", "BMD", "BND", "BOB", "BOV", "BRL", "BSD", "BTN", "BWP", "BYN", "BZD",
  "CAD", "CDF", "CHE", "CHF", "CHW", "CLP", "CNY", "COP", "COU", "CRC", "CUP", "CVE", "CZK",
  "DJF", "DKK", "DOP", "DZD", "EGP", "ERN", "ETB", "EUR", "FJD", "FKP", "GBP", "GEL", "GHS",
  "GIP", "GMD", "GNF", "GTQ", "GYD", "HKD", "HNL", "HTG", "HUF", "IDR", "ILS", "INR", "IQD",
  "IRR", "ISK", "JMD", "JOD", "JPY", "KES", "KGS", "KHR", "KMF", "KPW", "KRW", "KWD", "KYD",
  "KZT", "LAK", "LBP", "LKR", "LRD", "LSL", "LYD", "MAD", "MDL", "MGA", "MKD", "MMK", "MNT",
  "MOP", "MRU", "MUR", "MVR", "MWK", "MXN", "MXV", "MYR", "MZN", "NAD", "NGN", "NIO", "NOK",
  "NPR", "NZD", "OMR", "PAB", "PEN", "PGK", "PHP", "PKR", "PLN", "PYG", "QAR", "RON", "RSD",
  "RUB", "RWF", "SAR", "SBD", "SCR", "SDG", "SEK", "SGD", "SHP", "SLE", "SOS", "SRD", "SSP",
  "STN", "SVC", "SYP", "SZL", "THB", "TJS", "TMT", "TND", "TOP", "TRY", "TTD", "TWD", "TZS",
  "UAH", "UGX", "USD", "USN", "UYI", "UYU", "UZS", "VED", "VES", "VND", "VUV", "WST", "XAF",
  "XCD", "XOF", "XPF", "YER", "ZAR", "ZMW",
]);

export class MoneyInputError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "MoneyInputError";
  }
}

/// ISO-4217 minor-unit exponent. Returns undefined for an unknown code
/// outside CURRENCY_CODES; 0 for exponent-0 currencies; 3 for exponent-3;
/// and 2 for all other accepted codes.
export function currencyMinorExponent(code: string): number | undefined {
  const c = code.trim().toUpperCase();
  if (!CURRENCY_CODES.has(c)) return undefined;
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
  const exp = currencyMinorExponent(c) ?? 2;
  const rounded = Math.round(minor);
  if (exp === 0) {
    return `${rounded} ${c}`;
  }
  const sign = rounded < 0 ? "-" : "";
  const abs = Math.abs(rounded).toString().padStart(exp + 1, "0");
  const whole = abs.slice(0, -exp);
  const frac = abs.slice(-exp);
  return `${sign}${whole}.${frac} ${c}`;
}
