// The "Agree as listed" section of the listing form. It is a form section of
// its own so the listing screen stays a screen: the section owns the controls
// it reads back and the signing-status call that decides whether instant is
// offered.

import { checkbox, field, input, select, text } from "../dom";
import { t } from "../i18n/i18n";
import { call } from "../rpc";
import type { AsListedFields, Acceptance, PaymentTiming } from "./editor";

/// Builds the section and a reader for its current values.
export function asListedSection(isNew = true): {
  element: HTMLFieldSetElement;
  read: () => AsListedFields;
} {
  const element = document.createElement("fieldset");
  element.className = "listing-block block-as-listed";
  const title = document.createElement("legend");
  title.textContent = t("listing-as-listed-heading");

  const acceptance = select(
    [
      ["confirm-first", t("listing-acceptance-confirm")],
      ["instant", t("listing-acceptance-instant")],
    ],
    "confirm-first",
  );
  acceptance.className = "as-listed-acceptance";
  const signingNote = text("p", t("listing-instant-needs-signing"), "listing-instant-note");
  signingNote.hidden = true;
  let signingReady = false;
  void call<{ certificate?: { state?: string } }>("transaction.signing-status")
    .then((res) => {
      signingReady = res?.certificate?.state === "installed";
      const instant = Array.from(acceptance.options).find((o) => o.value === "instant");
      if (instant) instant.disabled = !signingReady;
      if (!signingReady && acceptance.value === "instant") {
        acceptance.value = "confirm-first";
      }
    })
    .catch(() => {
      const instant = Array.from(acceptance.options).find((o) => o.value === "instant");
      if (instant) instant.disabled = true;
    });
  acceptance.onchange = () => {
    signingNote.hidden = signingReady || acceptance.value !== "instant";
  };

  const notLimited = checkbox(t("listing-not-limited"), false);
  notLimited.box.className = "as-listed-not-limited";
  const instantCap = input(t("listing-instant-cap"), "20");
  instantCap.className = "as-listed-instant-cap";
  const quoteValidHours = input(t("listing-quote-valid-hours"), "");
  quoteValidHours.className = "as-listed-quote-hours";
  const paymentTiming = select(
    [
      ["none", t("listing-payment-timing-none")],
      ["before-work", t("listing-payment-timing-before")],
      ["after-work", t("listing-payment-timing-after")],
    ],
    "none",
  );
  paymentTiming.className = "as-listed-payment-timing";
  const cancellation = input(t("listing-deal-cancellation"));
  cancellation.className = "as-listed-cancellation";
  const refund = input(t("listing-deal-refund"));
  refund.className = "as-listed-refund";
  const dispute = input(t("listing-deal-dispute"));
  dispute.className = "as-listed-dispute";

  if (isNew) {
    populateDefaults(cancellation, refund, dispute, quoteValidHours);
  }

  element.append(
    title,
    field(t("listing-acceptance"), acceptance),
    signingNote,
    field(t("listing-not-limited"), notLimited.wrap),
    field(t("listing-instant-cap"), instantCap),
    field(t("listing-quote-valid-hours"), quoteValidHours),
    field(t("listing-payment-timing"), paymentTiming),
    field(t("listing-deal-cancellation"), cancellation),
    field(t("listing-deal-refund"), refund),
    field(t("listing-deal-dispute"), dispute),
  );

  return {
    element,
    read: () => ({
      acceptance: acceptance.value as Acceptance,
      not_limited: notLimited.box.checked,
      instant_cap: instantCap.value,
      quote_valid_hours: quoteValidHours.value.trim() || "1",
      payment_timing: paymentTiming.value as PaymentTiming,
      cancellation_terms: cancellation.value,
      refund_terms: refund.value,
      dispute_path: dispute.value,
    }),
  };
}

function populateDefaults(
  cancellation: HTMLInputElement,
  refund: HTMLInputElement,
  dispute: HTMLInputElement,
  quoteValidHours: HTMLInputElement,
) {
  void call<{
    cancellation_terms?: { text?: string } | string;
    refund_terms?: { text?: string } | string;
    dispute_path?: { text?: string } | string;
    quote_valid_secs?: number;
  } | null>("deal.defaults-get")
    .then((defaults) => {
      if (!defaults) return;
      const getTxt = (v: unknown) => {
        if (typeof v === "string") return v;
        if (typeof v === "object" && v !== null && "text" in v) return String((v as { text?: unknown }).text ?? "");
        return "";
      };
      const cTxt = getTxt(defaults.cancellation_terms);
      if (cTxt && cancellation.value.trim() === "") cancellation.value = cTxt;
      const rTxt = getTxt(defaults.refund_terms);
      if (rTxt && refund.value.trim() === "") refund.value = rTxt;
      const dTxt = getTxt(defaults.dispute_path);
      if (dTxt && dispute.value.trim() === "") dispute.value = dTxt;
      if (defaults.quote_valid_secs && quoteValidHours.value.trim() === "") {
        const hours = Math.floor(defaults.quote_valid_secs / 3600);
        const clamped = Math.max(1, Math.min(168, hours));
        quoteValidHours.value = String(clamped);
      }
    })
    .catch(() => {});
}

