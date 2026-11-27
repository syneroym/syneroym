import type { ServiceLocationKind } from "../listings/editor";
import { currencyMinorExponent, minorToDecimal } from "../money";

import { SCHEDULING_EXTENSION, SCHEDULING_SLOT_SCHEMA } from "../extensions/registry";

export interface CommitmentRowFields {
  giver: "quote-author" | "counterparty";
  kind: "service" | "goods" | "money";
  description?: string;
  declaredValueMinor: number;
  window?: {
    earliest_secs: number;
    latest_secs: number;
  };
  where?: ServiceLocationKind;
  address?: string;
  payee?: string;
  taxMinor?: number;
  feesMinor?: number;
  slotId?: string;
  listingId?: string;
  error?: string;
}

export interface QuoteFormFields {
  give?: string;
  scope: string;
  currency: string;
  amountMinor?: number;
  taxMinor?: number;
  feesMinor?: number;
  payee?: string;
  timing?: "before-work" | "after-work";
  where?: ServiceLocationKind;
  address?: string;
  cancellation?: string;
  refund?: string;
  dispute?: string;
  listingId?: string;
  slotId?: string;
  schedule?: {
    earliest_secs: number;
    latest_secs: number;
  };
  rows?: CommitmentRowFields[];
}

export function buildDealTerms(fields: QuoteFormFields, lang: string): Record<string, unknown> {
  const currency = fields.currency.trim().toUpperCase();
  const exp = currencyMinorExponent(currency) ?? 2;
  const isBefore = fields.timing === "before-work";
  const isAfter = fields.timing === "after-work";

  let commitments: Record<string, unknown>[];

  if (fields.rows && fields.rows.length > 0) {
    const moneyCounterpartyIds: string[] = [];
    const serviceAuthorIds: string[] = [];

    fields.rows.forEach((r, idx) => {
      const cid = `c${idx + 1}`;
      if (r.giver === "counterparty" && r.kind === "money") {
        moneyCounterpartyIds.push(cid);
      } else if (r.giver === "quote-author" && r.kind === "service") {
        serviceAuthorIds.push(cid);
      }
    });

    commitments = fields.rows.map((r) => {
      if (r.kind === "money") {
        const formattedAmount = minorToDecimal(r.declaredValueMinor, exp);
        return {
          giver: r.giver,
          resource_kind: "currency",
          kind: "currency",
          after: isAfter && r.giver === "counterparty" ? [...serviceAuthorIds] : [],
          description: {
            lang,
            text: `${currency} ${formattedAmount}`,
          },
          declared_value_minor: r.declaredValueMinor,
          payee: (r.payee ?? "").trim(),
          tax_minor: r.taxMinor ?? 0,
          fees_minor: r.feesMinor ?? 0,
        };
      }

      if (r.kind === "service") {
        const c: Record<string, unknown> = {
          giver: r.giver,
          resource_kind: r.slotId ? "service-in-slot" : "goods-or-service",
          kind: "fulfilment",
          after: isBefore && r.giver === "quote-author" ? [...moneyCounterpartyIds] : [],
          description: {
            lang,
            text: r.description?.trim() || fields.scope.trim(),
          },
          declared_value_minor: r.declaredValueMinor,
        };
        if (r.where || r.address) {
          c.location = r.address?.trim()
            ? { where: r.where ?? "at-receiver", address: r.address.trim() }
            : { where: r.where ?? "at-receiver" };
        }
        if (r.window) {
          c.window = r.window;
        }
        if (r.slotId) {
          c.extension = { id: SCHEDULING_EXTENSION, version: 1 };
          c.details = {
            extension: SCHEDULING_EXTENSION,
            schema: SCHEDULING_SLOT_SCHEMA,
            data: { listing_id: r.listingId ?? "", slot_id: r.slotId },
          };
        }
        return c;
      }

      // Goods
      const c: Record<string, unknown> = {
        giver: r.giver,
        resource_kind: "goods-or-service",
        kind: "fulfilment",
        after: [],
        description: {
          lang,
          text: r.description?.trim() || "",
        },
        declared_value_minor: r.declaredValueMinor,
      };
      if (r.window) {
        c.window = r.window;
      }
      if (r.where || r.address) {
        c.location = r.address?.trim()
          ? { where: r.where ?? "at-receiver", address: r.address.trim() }
          : { where: r.where ?? "at-receiver" };
      }
      return c;
    });
  } else {
    const amountMinor = fields.amountMinor ?? 0;
    const c1: Record<string, unknown> = {
      giver: "quote-author",
      resource_kind: fields.slotId ? "service-in-slot" : "goods-or-service",
      kind: "fulfilment",
      after: isBefore && amountMinor > 0 ? ["c2"] : [],
      description: {
        lang,
        text: fields.give?.trim() || fields.scope.trim(),
      },
      declared_value_minor: amountMinor,
      location: fields.address?.trim()
        ? { where: fields.where, address: fields.address.trim() }
        : { where: fields.where },
    };

    if (fields.schedule) {
      c1.window = fields.schedule;
    }
    if (fields.slotId) {
      c1.extension = { id: SCHEDULING_EXTENSION, version: 1 };
      c1.details = {
        extension: SCHEDULING_EXTENSION,
        schema: SCHEDULING_SLOT_SCHEMA,
        data: { listing_id: fields.listingId ?? "", slot_id: fields.slotId },
      };
    }

    commitments = [c1];

    if (amountMinor > 0) {
      const formattedAmount = minorToDecimal(amountMinor, exp);
      const c2: Record<string, unknown> = {
        giver: "counterparty",
        resource_kind: "currency",
        kind: "currency",
        after: !isBefore ? ["c1"] : [],
        description: {
          lang,
          text: `${currency} ${formattedAmount}`,
        },
        declared_value_minor: amountMinor,
        payee: (fields.payee ?? "").trim(),
        tax_minor: fields.taxMinor ?? 0,
        fees_minor: fields.feesMinor ?? 0,
      };
      commitments.push(c2);
    }
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
