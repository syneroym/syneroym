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
export type ServiceLocationKind = "at-provider" | "at-customer" | "remote";
export type AddressDisclosure = "on-agreement" | "public";
export type OpenTo = "anyone" | "members" | "referral" | "existing-customers";
export type ListingStatus = "draft" | "active" | "withdrawn";

/// One editor block, plus the checkbox that says whether the provider filled it in.
export interface BlockToggle<T> {
  enabled: boolean;
  value: T;
}

export interface BookingFields {
  mode: BookingMode;
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
  methods: string;
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
  /// A single human-readable service area label. Geometry is C6's.
  area_label: string;
}

export interface RelationshipFields {
  open_to: OpenTo;
  member_of: string;
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
  categories: string;
  conversation_address: string;
  status: ListingStatus;
  booking: BlockToggle<BookingFields>;
  payment: BlockToggle<PaymentFields>;
  product: BlockToggle<ProductFields>;
  service: BlockToggle<ServiceFields>;
  location: BlockToggle<LocationFields>;
  relationship: BlockToggle<RelationshipFields>;
  service_record: BlockToggle<ServiceRecordFields>;
}

export class ListingInputError extends Error {}

/// "35.50" -> 3550, "35" -> 3500, "35.5" -> 3550, "" -> undefined.
/// Rejects anything that is not a plain non-negative amount with at most
/// two decimal places, so the result is always an integer number of minor
/// units.
export function toMinorUnits(input: string): number | undefined {
  const t = input.trim();
  if (t === "") return undefined;
  const m = /^(\d+)(?:\.(\d{1,2}))?$/.exec(t);
  if (!m) {
    throw new ListingInputError(`"${input}" is not an amount like 12 or 12.50`);
  }
  const minor = m[1] + (m[2] ?? "").padEnd(2, "0");
  return Number.parseInt(minor, 10);
}

/// "48.8566" -> 48856600, "-2.35" -> -2350000, "48" -> 48000000.
/// At most six decimal places (a micro-degree is about 11 cm); the result
/// is always an integer.
export function toMicroDegrees(input: string): number {
  const t = input.trim();
  const m = /^(-?)(\d+)(?:\.(\d{1,6}))?$/.exec(t);
  if (!m) {
    throw new ListingInputError(`"${input}" is not a coordinate in degrees`);
  }
  const micros = m[2] + (m[3] ?? "").padEnd(6, "0");
  const magnitude = Number.parseInt(micros, 10);
  return m[1] === "-" ? -magnitude : magnitude;
}

/// A whole non-negative count. "" -> undefined (the caller decides whether
/// that is allowed); a decimal or a negative value is rejected.
export function toWholeCount(input: string, field: string): number | undefined {
  const t = input.trim();
  if (t === "") return undefined;
  if (!/^\d+$/.test(t)) {
    throw new ListingInputError(`${field} must be a whole number of seconds or units`);
  }
  return Number.parseInt(t, 10);
}

function requireWhole(input: string, field: string): number {
  const v = toWholeCount(input, field);
  if (v === undefined) throw new ListingInputError(`${field} is required`);
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

function bookingBlock(f: BookingFields): Block {
  return {
    mode: f.mode,
    lead_time_secs: requireWhole(f.lead_time_secs, "lead time"),
    cancellation_window_secs: requireWhole(f.cancellation_window_secs, "cancellation window"),
    max_per_booking: requireWhole(f.max_per_booking, "max per booking"),
  };
}

function paymentBlock(f: PaymentFields): Block {
  const block: Block = {
    currency: f.currency.trim().toUpperCase(),
    model: f.model,
    tax_included: f.tax_included,
    methods: tokens(f.methods),
    payee: f.payee.trim(),
  };
  const amount = toMinorUnits(f.amount);
  if (f.model !== "quote-only") {
    if (amount === undefined) {
      throw new ListingInputError("an amount is required unless the model is quote-only");
    }
    block.amount_minor = amount;
  } else if (amount !== undefined) {
    throw new ListingInputError("a quote-only listing must not carry an amount");
  }
  const fees = toMinorUnits(f.fees);
  if (fees !== undefined) block.fees_minor = fees;
  return block;
}

function productBlock(f: ProductFields): Block {
  const block: Block = {
    unit: f.unit.trim(),
    pack_size: requireWhole(f.pack_size, "pack size"),
    condition: f.condition,
  };
  if (f.sku.trim()) block.sku = f.sku.trim();
  return block;
}

function serviceBlock(f: ServiceFields): Block {
  return {
    duration_secs: requireWhole(f.duration_secs, "duration"),
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
    throw new ListingInputError("a members-only listing needs the group DID");
  }
  return block;
}

function serviceRecordBlock(f: ServiceRecordFields): Block {
  return {
    issues_fulfilment_receipt: f.issues_fulfilment_receipt,
    warranty_secs: requireWhole(f.warranty_secs, "warranty"),
    retention_secs: requireWhole(f.retention_secs, "retention"),
  };
}

/// The `listing.set` params for this form. Every number in the result is an
/// integer; blocks whose checkbox is off are omitted entirely.
export function buildSetListingParams(form: ListingForm): Record<string, unknown> {
  const title = form.title.trim();
  if (!title) throw new ListingInputError("a title is required");

  const params: Record<string, unknown> = {
    title,
    summary: form.summary.trim(),
    categories: tokens(form.categories).map((c) => c.toLowerCase()),
    status: form.status,
  };
  const slug = form.slug.trim();
  if (slug) params.slug = slug;
  const address = form.conversation_address.trim();
  if (address) params.conversation_address = address;

  if (form.booking.enabled) params.booking = bookingBlock(form.booking.value);
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
    throw new ListingInputError("a slot needs a start and an end time");
  }
  if (end <= start) {
    throw new ListingInputError("a slot's end must be after its start");
  }
  const cap = toWholeCount(capacity, "capacity");
  if (cap === undefined || cap < 1) {
    throw new ListingInputError("a slot needs a capacity of at least 1");
  }
  return {
    start_secs: Math.floor(start / 1000),
    end_secs: Math.floor(end / 1000),
    capacity: cap,
  };
}
