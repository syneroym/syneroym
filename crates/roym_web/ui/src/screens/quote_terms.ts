import type { ServiceLocationKind } from "../listings/editor";
import { currencyMinorExponent, minorToDecimal } from "../money";

export interface QuoteFormFields {
  give?: string;
  scope: string;
  currency: string;
  amountMinor: number;
  taxMinor?: number;
  feesMinor?: number;
  payee: string;
  timing: "before-work" | "after-work";
  where: ServiceLocationKind;
  address?: string;
  cancellation?: string;
  refund?: string;
  dispute?: string;
  slotId?: string;
  schedule?: {
    earliest_secs: number;
    latest_secs: number;
  };
}

export function buildDealTerms(fields: QuoteFormFields, lang: string): Record<string, unknown> {
  const currency = fields.currency.trim().toUpperCase();
  const exp = currencyMinorExponent(currency) ?? 2;
  const isBefore = fields.timing === "before-work";

  const c1: Record<string, unknown> = {
    giver: "quote-author",
    resource_kind: fields.slotId ? "service-in-slot" : "goods-or-service",
    kind: "fulfilment",
    after: isBefore && fields.amountMinor > 0 ? ["c2"] : [],
    description: {
      lang,
      text: fields.give?.trim() || fields.scope.trim(),
    },
    declared_value_minor: fields.amountMinor,
    location: fields.address?.trim()
      ? { where: fields.where, address: fields.address.trim() }
      : { where: fields.where },
  };

  if (fields.schedule) {
    c1.window = fields.schedule;
  }
  if (fields.slotId) {
    c1.extension = { id: "scheduling", version: 1 };
    c1.details = {
      extension: "scheduling",
      schema: "scheduling/slot@1",
      data: { slot_id: fields.slotId },
    };
  }

  const commitments: Record<string, unknown>[] = [c1];

  if (fields.amountMinor > 0) {
    const formattedAmount = minorToDecimal(fields.amountMinor, exp);
    const c2: Record<string, unknown> = {
      giver: "counterparty",
      resource_kind: "currency",
      kind: "currency",
      after: !isBefore ? ["c1"] : [],
      description: {
        lang,
        text: `${currency} ${formattedAmount}`,
      },
      declared_value_minor: fields.amountMinor,
      payee: fields.payee.trim(),
      tax_minor: fields.taxMinor ?? 0,
      fees_minor: fields.feesMinor ?? 0,
    };
    commitments.push(c2);
  }

  return {
    currency,
    commitments,
    scope: { lang, text: fields.scope.trim() },
    cancellation_terms: { lang, text: (fields.cancellation ?? "").trim() },
    refund_terms: { lang, text: (fields.refund ?? "").trim() },
    dispute_path: { lang, text: (fields.dispute ?? "").trim() },
  };
}
