/// Freshness shown to the person, always spelled in words. `secs` is an
/// age the caller already computed against this person's own clock.
export function ageWords(secs: number): string {
  if (secs < 90) return "moments ago";
  const mins = Math.round(secs / 60);
  if (mins < 90) return `${mins} minutes ago`;
  const hours = Math.round(secs / 3600);
  if (hours < 48) return `${hours} hours ago`;
  return `${Math.round(secs / 86400)} days ago`;
}

/// A calendar date in UTC, spelled the same for everyone (`2026-09-29`).
export function dateWords(secs: number): string {
  return new Date(secs * 1000).toISOString().slice(0, 10);
}
