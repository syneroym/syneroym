import type { ServiceLocationKind } from "../listings/editor";

export interface FormValues {
  currency: string;
  amountMinor?: number;
  timing: "before-work" | "after-work";
  where: ServiceLocationKind;
  address: string;
  payee: string;
  cancellation: string;
  refund: string;
  dispute: string;
  quoteValidSecs: number;
}

export interface ListingLike {
  payment?: {
    currency?: string;
    amount_minor?: number;
    timing?: "before-work" | "after-work";
    payee?: string;
    [key: string]: unknown;
  };
  price?: {
    currency?: string;
    amount_minor?: number;
    timing?: "before-work" | "after-work";
    payee?: string;
    [key: string]: unknown;
  };
  deal_terms?: {
    payment_timing?: "before-work" | "after-work";
    cancellation_terms?: { text?: string } | string;
    refund_terms?: { text?: string } | string;
    dispute_path?: { text?: string } | string;
    quote_valid_secs?: number;
    [key: string]: unknown;
  };
  deal?: {
    payment_timing?: "before-work" | "after-work";
    cancellation_terms?: { text?: string } | string;
    refund_terms?: { text?: string } | string;
    dispute_path?: { text?: string } | string;
    quote_valid_secs?: number;
    [key: string]: unknown;
  };
  location?: {
    where?: ServiceLocationKind;
    address?: string;
    [key: string]: unknown;
  };
  [key: string]: unknown;
}

export interface SavedDefaultsLike {
  cancellation_terms?: { text?: string } | string;
  refund_terms?: { text?: string } | string;
  dispute_path?: { text?: string } | string;
  quote_valid_secs?: number;
  [key: string]: unknown;
}

export interface BuiltInDefaultsLike {
  authorDid?: string;
  cancellation?: string;
  refund?: string;
  dispute?: string;
}

function textOf(val: unknown): string {
  if (typeof val === "string") return val.trim();
  if (typeof val === "object" && val !== null && "text" in val) {
    return String((val as { text?: unknown }).text ?? "").trim();
  }
  return "";
}

/**
 * Pure function implementing the fill order for the quote form.
 * 1. From listing
 * 2. Then from saved deal defaults
 * 3. Then from built-in text
 * 4. Currency falls back to INR; payee falls back to author DID.
 */
export function fillQuoteForm(
  listing?: ListingLike | null,
  saved?: SavedDefaultsLike | null,
  builtIn?: BuiltInDefaultsLike | null,
): FormValues {
  const payment = listing?.payment || listing?.price;
  const dealTerms = listing?.deal_terms || listing?.deal;

  const currency = payment?.currency?.trim() || "INR";
  const amountMinor = payment?.amount_minor;

  const timing =
    dealTerms?.payment_timing ||
    payment?.timing ||
    "after-work";

  const where: ServiceLocationKind =
    listing?.location?.where || "at-receiver";
  const address = listing?.location?.address?.trim() || "";

  const payee =
    payment?.payee?.trim() ||
    builtIn?.authorDid?.trim() ||
    "";

  const cancellation =
    textOf(dealTerms?.cancellation_terms) ||
    textOf(saved?.cancellation_terms) ||
    (builtIn?.cancellation?.trim() ?? "");

  const refund =
    textOf(dealTerms?.refund_terms) ||
    textOf(saved?.refund_terms) ||
    (builtIn?.refund?.trim() ?? "");

  const dispute =
    textOf(dealTerms?.dispute_path) ||
    textOf(saved?.dispute_path) ||
    (builtIn?.dispute?.trim() ?? "");

  const quoteValidSecs =
    dealTerms?.quote_valid_secs ||
    saved?.quote_valid_secs ||
    86400;

  return {
    currency,
    amountMinor,
    timing,
    where,
    address,
    payee,
    cancellation,
    refund,
    dispute,
    quoteValidSecs,
  };
}
