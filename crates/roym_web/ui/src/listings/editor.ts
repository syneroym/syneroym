// Pure value mapping for the listing editor.
//
// A signed listing payload may hold no number that is not an integer: the
// host refuses a decimal before it signs. So the editor takes human input
// (a price like "35.50", a latitude like "48.8566") and converts it to the
// integer shape the service expects -- minor currency units and
// micro-degrees -- and never emits a fractional number.

export type BookingMode = "slots" | "order" | "enquiry";
export type PaymentModel = "fixed" | "per-hour" | "per-unit" | "quote-only";
export type ProductCondition = "new" | "used" | "refurbished";
export type ServiceLocationKind = "at-giver" | "at-receiver" | "remote";
export type AddressDisclosure = "on-agreement" | "public";
export type OpenTo = "anyone" | "members" | "referral" | "past-counterparties";
export type ListingStatus = "draft" | "active" | "withdrawn";
export type Acceptance = "instant" | "confirm-first";
export type PaymentTiming = "before-work" | "after-work" | "none";

/// One editor block, plus the checkbox that says whether the listing owner filled it in.
export interface BlockToggle<T> {
  enabled: boolean;
  value: T;
}

export interface BookingFields {
  lead_time_secs: string;
  cancellation_window_secs: string;
  max_per_booking: string;
}

export interface PaymentFields {
  currency: string;
  model: PaymentModel;
  /// Entered as a decimal string like "35.50"; mapped to minor units.
  amount: string;
  tax_included: boolean;
  fees: string;
  methods: string[];
  payee: string;
}

export interface ProductFields {
  unit: string;
  pack_size: string;
  condition: ProductCondition;
  sku: string;
}

export interface ServiceFields {
  duration_secs: string;
  includes: string;
  excludes: string;
  prerequisites: string;
}

export interface LocationFields {
  where: ServiceLocationKind;
  address_disclosure: AddressDisclosure;
  // A single human-readable service area label; geometric areas come later.
  area_label: string;
}

export interface RelationshipFields {
  open_to: OpenTo;
  member_of: string;
}

/// The "Agree as listed" section: how the owner answers a tap, how long a
/// quote built from the listing stays valid, and the texts it carries.
export interface AsListedFields {
  acceptance: Acceptance;
  not_limited: boolean;
  instant_cap: string;
  quote_valid_hours: string;
  payment_timing: PaymentTiming;
  cancellation_terms: string;
  refund_terms: string;
  dispute_path: string;
}

export interface ServiceRecordFields {
  issues_fulfilment_receipt: boolean;
  warranty_secs: string;
  retention_secs: string;
}

export interface ListingForm {
  slug: string;
  title: string;
  summary: string;
  lang?: string;
  translations?: Array<{ lang: string; title: string; summary: string }>;
  categories: string[];
  conversation_address: string;
  status: ListingStatus;
  /// How the listing is booked. It names the extension that the listing
  /// carries: slots are the scheduling extension, an order is one-shot, and
  /// an enquiry names none.
  mode: BookingMode;
  booking: BlockToggle<BookingFields>;
  payment: BlockToggle<PaymentFields>;
  product: BlockToggle<ProductFields>;
  service: BlockToggle<ServiceFields>;
  location: BlockToggle<LocationFields>;
  relationship: BlockToggle<RelationshipFields>;
  service_record: BlockToggle<ServiceRecordFields>;
  as_listed: AsListedFields;
}

import {
  EXPONENT_0,
  EXPONENT_3,
  MoneyInputError,
  currencyMinorExponent,
  formatMinor,
  toMinorUnits,
} from "../money.js";
import { type ExtensionRef, ONE_SHOT_REF, SCHEDULING_REF } from "../extensions/registry";
import { HubError } from "../hub-error";
import { currentLanguage, t } from "../i18n/i18n";

export class ListingInputError extends HubError {
  constructor(message: string) {
    super(message);
    this.name = "ListingInputError";
  }
}

export {
  EXPONENT_0,
  EXPONENT_3,
  MoneyInputError,
  currencyMinorExponent,
  formatMinor,
  toMinorUnits,
};

/// "48.8566" -> 48856600, "-2.35" -> -2350000, "48" -> 48000000.
/// At most six decimal places (a micro-degree is about 11 cm); the result
/// is always an integer.
export function toMicroDegrees(input: string): number {
  const trimmed = input.trim();
  const m = /^(-?)(\d+)(?:\.(\d{1,6}))?$/.exec(trimmed);
  if (!m) {
    throw new ListingInputError(t("listing-error-coordinate", { input }));
  }
  const micros = m[2] + (m[3] ?? "").padEnd(6, "0");
  const magnitude = Number.parseInt(micros, 10);
  return m[1] === "-" ? -magnitude : magnitude;
}

/// A whole non-negative count. "" -> undefined (the caller decides whether
/// that is allowed); a decimal or a negative value is rejected.
export function toWholeCount(input: string, field: string): number | undefined {
  const trimmed = input.trim();
  if (trimmed === "") return undefined;
  if (!/^\d+$/.test(trimmed)) {
    throw new ListingInputError(t("listing-error-whole", { field }));
  }
  return Number.parseInt(trimmed, 10);
}

function requireWhole(input: string, field: string): number {
  const v = toWholeCount(input, field);
  if (v === undefined) throw new ListingInputError(t("listing-error-required", { field }));
  return v;
}

/// Splits a comma-separated field into trimmed, non-empty tokens.
export function tokens(input: string): string[] {
  return input
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

type Block = Record<string, unknown>;

/// The extension a booking mode names, or `undefined` for an enquiry.
export function extensionForMode(mode: BookingMode): ExtensionRef | undefined {
  switch (mode) {
    case "slots":
      return SCHEDULING_REF;
    case "order":
      return ONE_SHOT_REF;
    case "enquiry":
      return undefined;
  }
}

function bookingBlock(f: BookingFields): Block {
  return {
    lead_time_secs: requireWhole(f.lead_time_secs, t("listing-field-lead-time")),
    cancellation_window_secs: requireWhole(f.cancellation_window_secs, t("listing-field-cancellation-window")),
    max_per_booking: requireWhole(f.max_per_booking, t("listing-field-max-per-booking")),
  };
}

function parseMoney(input: string, exponent: number): number | undefined {
  try {
    return toMinorUnits(input, exponent);
  } catch (err) {
    if (err instanceof MoneyInputError) {
      throw new ListingInputError(err.message);
    }
    throw err;
  }
}

function paymentBlock(f: PaymentFields): Block {
  const currency = f.currency.trim().toUpperCase();
  const block: Block = {
    currency,
    model: f.model,
    tax_included: f.tax_included,
    methods: f.methods,
    payee: f.payee.trim(),
  };
  const exponent = currencyMinorExponent(currency);
  if (exponent === undefined) {
    throw new ListingInputError(t("listing-error-currency", { currency }));
  }
  const amount = parseMoney(f.amount, exponent);
  if (f.model !== "quote-only") {
    if (amount === undefined) {
      throw new ListingInputError(t("listing-error-amount-required"));
    }
    block.amount_minor = amount;
  } else if (amount !== undefined) {
    throw new ListingInputError(t("listing-error-quote-only-amount"));
  }
  const fees = parseMoney(f.fees, exponent);
  if (fees !== undefined) block.fees_minor = fees;
  return block;
}

function productBlock(f: ProductFields): Block {
  const block: Block = {
    unit: f.unit.trim(),
    pack_size: requireWhole(f.pack_size, t("listing-field-pack-size")),
    condition: f.condition,
  };
  if (f.sku.trim()) block.sku = f.sku.trim();
  return block;
}

function serviceBlock(f: ServiceFields): Block {
  return {
    duration_secs: requireWhole(f.duration_secs, t("listing-field-duration")),
    includes: tokens(f.includes),
    excludes: tokens(f.excludes),
    prerequisites: tokens(f.prerequisites),
  };
}

function locationBlock(f: LocationFields): Block {
  const block: Block = {
    where: f.where,
    address_disclosure: f.address_disclosure,
  };
  const label = f.area_label.trim();
  if (label) {
    block.service_area = [{ kind: "named", label }];
  }
  return block;
}

function relationshipBlock(f: RelationshipFields): Block {
  const block: Block = { open_to: f.open_to };
  const memberOf = f.member_of.trim();
  if (memberOf) block.member_of = memberOf;
  else if (f.open_to === "members") {
    throw new ListingInputError(t("listing-error-members-group"));
  }
  return block;
}

function serviceRecordBlock(f: ServiceRecordFields): Block {
  return {
    issues_fulfilment_receipt: f.issues_fulfilment_receipt,
    warranty_secs: requireWhole(f.warranty_secs, t("listing-field-warranty")),
    retention_secs: requireWhole(f.retention_secs, t("listing-field-retention")),
  };
}

/// The deal-terms block, or `undefined` when the owner left it empty. A
/// listing with no block cannot be agreed as listed; the three texts must be
/// filled together, because the node refuses an empty one.
function asListedBlock(f: AsListedFields, lang: string): Block | undefined {
  const texts = [f.cancellation_terms, f.refund_terms, f.dispute_path].map((s) => s.trim());
  if (texts.every((body) => body === "")) return undefined;
  if (texts.some((body) => body === "")) {
    throw new ListingInputError(t("listing-error-deal-terms-required"));
  }
  const hours = requireWhole(f.quote_valid_hours, t("listing-quote-valid-hours"));
  if (hours === 0 || hours > 168) {
    throw new ListingInputError(t("listing-error-quote-valid-hours"));
  }
  const block: Block = {
    cancellation_terms: { lang, text: texts[0] },
    refund_terms: { lang, text: texts[1] },
    dispute_path: { lang, text: texts[2] },
    quote_valid_secs: hours * 3_600,
  };
  if (f.payment_timing !== "none") block.payment_timing = f.payment_timing;
  return block;
}

/// The `listing.set` params for this form. Every number in the result is an
/// integer; blocks whose checkbox is off are omitted entirely.
export function buildSetListingParams(form: ListingForm): Record<string, unknown> {
  const title = form.title.trim();
  if (!title) throw new ListingInputError(t("listing-error-title-required"));

  const lang = form.lang || currentLanguage();
  const params: Record<string, unknown> = {
    title,
    summary: form.summary.trim(),
    lang,
    categories: form.categories,
    status: form.status,
  };

  params.acceptance = form.as_listed.acceptance;
  params.not_limited = form.as_listed.not_limited;
  params.instant_cap = requireWhole(form.as_listed.instant_cap, t("listing-instant-cap"));
  const dealTerms = asListedBlock(form.as_listed, lang);
  if (dealTerms !== undefined) params.deal_terms = dealTerms;

  if (form.translations && form.translations.length > 0) {
    const seenLangs = new Set<string>();
    const translations = [];
    for (const tr of form.translations) {
      const trTitle = tr.title.trim();
      if (!trTitle) {
        throw new ListingInputError(t("listing-error-translation-title-required"));
      }
      if (tr.lang === lang) {
        throw new ListingInputError(t("listing-error-translation-same-language"));
      }
      if (seenLangs.has(tr.lang)) {
        throw new ListingInputError(t("listing-error-translation-duplicate"));
      }
      seenLangs.add(tr.lang);
      translations.push({
        lang: tr.lang,
        title: trTitle,
        summary: tr.summary.trim(),
      });
    }
    params.translations = translations;
  }
  const slug = form.slug.trim();
  if (slug) params.slug = slug;
  const address = form.conversation_address.trim();
  if (address) params.conversation_address = address;

  const extension = extensionForMode(form.mode);
  if (extension) params.extension = extension;
  // Slots need lead time, a cancellation window and a seat limit.
  if (form.booking.enabled || form.mode === "slots") {
    params.booking = bookingBlock(form.booking.value);
  }
  if (form.payment.enabled) params.payment = paymentBlock(form.payment.value);
  if (form.product.enabled) params.product = productBlock(form.product.value);
  if (form.service.enabled) params.service = serviceBlock(form.service.value);
  if (form.location.enabled) params.location = locationBlock(form.location.value);
  if (form.relationship.enabled) params.relationship = relationshipBlock(form.relationship.value);
  if (form.service_record.enabled) {
    params.service_record = serviceRecordBlock(form.service_record.value);
  }
  return params;
}

/// A slot's integer-second bounds from two `datetime-local` values.
export function slotFromLocalDatetimes(
  startLocal: string,
  endLocal: string,
  capacity: string,
): { start_secs: number; end_secs: number; capacity: number } {
  const start = Date.parse(startLocal);
  const end = Date.parse(endLocal);
  if (Number.isNaN(start) || Number.isNaN(end)) {
    throw new ListingInputError(t("listing-error-slot-times"));
  }
  if (end <= start) {
    throw new ListingInputError(t("listing-error-slot-order"));
  }
  const cap = toWholeCount(capacity, t("listing-field-capacity"));
  if (cap === undefined || cap < 1) {
    throw new ListingInputError(t("listing-error-slot-capacity"));
  }
  return {
    start_secs: Math.floor(start / 1000),
    end_secs: Math.floor(end / 1000),
    capacity: cap,
  };
}
