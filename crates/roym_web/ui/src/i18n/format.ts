import { formatLocale, t } from "./i18n";

const cache = new Map<string, unknown>();

/// One `Intl` object per kind and format locale.
export function cached<T>(kind: string, make: (locale: string) => T): T {
  const locale = formatLocale();
  const key = `${kind}|${locale}`;
  let value = cache.get(key) as T | undefined;
  if (value === undefined) {
    value = make(locale);
    cache.set(key, value);
  }
  return value;
}

export function formatNumber(n: number): string {
  return cached("number", (l) => new Intl.NumberFormat(l)).format(n);
}

/// The date for a time in milliseconds, or `undefined` when the value is not
/// a date. `Intl` throws for such a value, and a time from another node must
/// not be able to stop a whole screen from drawing.
function validDate(ms: number): Date | undefined {
  const date = new Date(ms);
  return Number.isFinite(date.getTime()) ? date : undefined;
}

/// Local time with the zone name. A value that is not a date gives the
/// `date-invalid` text, as `toLocaleString()` printed `Invalid Date`.
export function formatDateTime(secs: number): string {
  const date = validDate(secs * 1000);
  if (date === undefined) return t("date-invalid");
  return cached(
    "datetime",
    (l) =>
      new Intl.DateTimeFormat(l, {
        year: "numeric",
        month: "short",
        day: "numeric",
        hour: "numeric",
        minute: "2-digit",
        timeZoneName: "short",
      }),
  ).format(date);
}

export function formatTime(ms: number): string {
  const date = validDate(ms);
  if (date === undefined) return t("date-invalid");
  return cached(
    "time",
    (l) => new Intl.DateTimeFormat(l, { hour: "numeric", minute: "2-digit" }),
  ).format(date);
}

/// A calendar date in UTC, the same date for everyone.
export function formatUtcDate(secs: number): string {
  return cached(
    "utc-date",
    (l) => new Intl.DateTimeFormat(l, { dateStyle: "medium", timeZone: "UTC" }),
  ).format(new Date(secs * 1000));
}

/// Freshness in words. `secs` is an age the caller already computed
/// against this person's own clock.
export function formatAge(secs: number): string {
  if (!Number.isFinite(secs)) return t("date-invalid");
  if (secs < 90) return t("age-moments");
  const relative = cached(
    "relative",
    (l) => new Intl.RelativeTimeFormat(l, { numeric: "always" }),
  );
  const mins = Math.round(secs / 60);
  if (mins < 90) return relative.format(-mins, "minute");
  const hours = Math.round(secs / 3600);
  if (hours < 48) return relative.format(-hours, "hour");
  return relative.format(-Math.round(secs / 86400), "day");
}

/// A `<time>` element whose `datetime` the Hub computes from a number, never
/// from a string of a card. Throws a `RangeError` for a value that is not a
/// date, as `Date.toISOString()` does, so a card row behaves as it always did.
export function timeElement(secs: number): HTMLTimeElement {
  const el = document.createElement("time");
  el.dateTime = new Date(secs * 1000).toISOString();
  el.textContent = formatDateTime(secs);
  return el;
}
