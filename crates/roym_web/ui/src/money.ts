import { cached } from "./i18n/format";
import { formatLocale, t } from "./i18n/i18n";

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
  const text = input.trim();
  if (text === "") return undefined;
  const frac = exponent > 0 ? `(?:\\.(\\d{1,${exponent}}))?` : "";
  const m = new RegExp(`^(\\d+)${frac}$`).exec(text);
  if (!m) {
    const example = exponent > 0 ? `12 or 12.${"5".padEnd(exponent, "0")}` : "12";
    throw new MoneyInputError(t("money-input-invalid", { input, example }));
  }
  const minor = m[1] + (m[2] ?? "").padEnd(exponent, "0");
  return Number.parseInt(minor, 10);
}

/// Integer minor units as an exact decimal string, e.g. 3550, 2 -> "35.50".
/// A string keeps every digit of a large amount, which a float would not.
function minorToDecimal(minor: number, exp: number): `${number}` {
  if (exp === 0) return `${minor}`;
  const sign = minor < 0 ? "-" : "";
  const abs = Math.abs(minor).toString().padStart(exp + 1, "0");
  return `${sign}${abs.slice(0, -exp)}.${abs.slice(-exp)}` as `${number}`;
}

/// Formats integer minor units for display in the Hub language's money
/// style, e.g. 3550 USD -> "$35.50" and 10000050 INR -> "₹1,00,000.50".
/// The exponent is the Rust-pinned one, so the digits never change with
/// the browser's currency data. A code `Intl` rejects (one from another
/// node) is shown as `<amount> <code>`.
export function formatMinor(minor: number, currency: string): string {
  const code = currency.trim().toUpperCase();
  const exp = currencyMinorExponent(code) ?? 2;
  const decimal = minorToDecimal(Math.round(minor), exp);
  const digits = { minimumFractionDigits: exp, maximumFractionDigits: exp };
  try {
    return cached(
      `money|${code}|${exp}`,
      (locale) => new Intl.NumberFormat(locale, { style: "currency", currency: code, ...digits }),
    ).format(decimal);
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
    return `${new Intl.NumberFormat(formatLocale(), digits).format(decimal)} ${code}`;
  }
}
