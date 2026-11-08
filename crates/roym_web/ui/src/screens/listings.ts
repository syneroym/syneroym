import {
  buildSetListingParams,
  slotFromLocalDatetimes,
  type ListingForm,
  type OpenTo,
  type ServiceLocationKind,
} from "../listings/editor";
import {
  OPEN_TO_KINDS,
  openToLabel,
  serviceLocationEditorLabel,
  SERVICE_LOCATION_KINDS,
} from "../listings/labels";
import { publishRefusalWords } from "../directory/membership";
import { checkbox, codeOptions, errText, field, input, refiller, select, text } from "../dom";
import { formatDateTime } from "../i18n/format";
import { codeWord, messageId, t } from "../i18n/i18n";
import { call } from "../rpc";
import { categoryPicker } from "../taxonomy/category_picker";
import { methodLabel, PAYMENT_METHODS, unitLabel, UNITS } from "../taxonomy/taxonomy";

interface ListingListRow {
  listing_id: string;
  slug: string;
  title: string;
  status: string;
  record_id: string;
  updated_at_secs: number;
  version_count: number;
}

/// Message id for each code shown as a word. A `Map`, so a code from
/// another node can never hit an inherited property; a code not in a table
/// is shown as it came.
const STATUS_LABELS = new Map<string, string>([
  ["active", messageId("listing-status-active")],
  ["draft", messageId("listing-status-draft")],
  ["withdrawn", messageId("listing-status-withdrawn")],
]);
const BOOKING_MODE_LABELS = new Map<string, string>([
  ["enquiry", messageId("listing-mode-enquiry")],
  ["slots", messageId("listing-mode-slots")],
  ["order", messageId("listing-mode-order")],
]);
const PRICING_MODEL_LABELS = new Map<string, string>([
  ["per-hour", messageId("listing-model-per-hour")],
  ["fixed", messageId("listing-model-fixed")],
  ["per-unit", messageId("listing-model-per-unit")],
  ["quote-only", messageId("listing-model-quote-only")],
]);
const CONDITION_LABELS = new Map<string, string>([
  ["new", messageId("listing-condition-new")],
  ["used", messageId("listing-condition-used")],
  ["refurbished", messageId("listing-condition-refurbished")],
]);
const DISCLOSURE_LABELS = new Map<string, string>([
  ["on-agreement", messageId("listing-disclosure-on-agreement")],
  ["public", messageId("listing-disclosure-public")],
]);

/// A collapsed `<details>` block with an "included" checkbox in the summary.
/// `kind` keeps the CSS class; `label` is the text shown.
function block(kind: string, label: string): { details: HTMLDetailsElement; enabled: HTMLInputElement; body: HTMLElement } {
  const details = document.createElement("details");
  details.className = `listing-block block-${kind}`;
  const summary = document.createElement("summary");
  const enabled = document.createElement("input");
  enabled.type = "checkbox";
  enabled.className = "block-enabled";
  enabled.onclick = (e) => e.stopPropagation();
  // Ticking "include this block" reveals its fields; unticking collapses them.
  enabled.onchange = () => {
    details.open = enabled.checked;
  };
  summary.append(enabled, text("span", label));
  details.appendChild(summary);
  const body = document.createElement("div");
  body.className = "block-body";
  details.appendChild(body);
  return { details, enabled, body };
}

export async function renderListings(container: HTMLElement) {
  container.replaceChildren();
  const box = document.createElement("div");
  box.className = "listings-screen";
  box.appendChild(text("h2", t("listing-screen-title")));

  const listHost = document.createElement("div");
  listHost.className = "listing-list";
  box.appendChild(listHost);

  const editor = buildEditor(() => reload());
  box.appendChild(editor.root);

  box.appendChild(buildVerifyBox());

  const reload = refiller(listHost, async (into) => {
    into.appendChild(text("h3", t("listing-your-heading")));
    let rows: ListingListRow[] = [];
    try {
      const res = await call<{ listings: ListingListRow[] }>("listing.list");
      rows = res.listings;
    } catch (err) {
      into.appendChild(text("p", t("listing-load-failed", { detail: errText(err) })));
      return;
    }
    if (rows.length === 0) {
      into.appendChild(text("p", t("listing-empty")));
    }
    for (const row of rows) {
      into.appendChild(listingRow(row, reload));
    }
  });

  await reload();
  container.appendChild(box);
}

function listingRow(row: ListingListRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "listing-row";
  wrap.dataset.status = row.status;

  wrap.appendChild(text("span", row.title || row.slug, "listing-title"));
  wrap.appendChild(text("span", row.slug, "listing-slug"));
  wrap.appendChild(
    text("span", codeWord(STATUS_LABELS, row.status), `listing-status status-${row.status}`),
  );
  wrap.appendChild(
    text("span", t("listing-version", { count: String(row.version_count) }), "listing-version"),
  );

  const historyBtn = text("button", t("listing-history-button"), "button listing-history-btn") as HTMLButtonElement;
  const historyHost = document.createElement("div");
  historyHost.className = "listing-history";
  const fillHistory = refiller(historyHost, async (into: DocumentFragment, show: boolean) => {
    if (!show) return;
    try {
      const res = await call<{ history: string[] }>("listing.history", {
        listing_id: row.listing_id,
      });
      res.history.forEach((envStr, i) => {
        const env = JSON.parse(envStr) as {
          record_id?: string;
          supersedes?: string | null;
          payload?: { title?: string };
        };
        const n = String(i + 1);
        const id = env.record_id ?? "";
        const line = env.supersedes
          ? t("listing-history-line", { n, id, previous: env.supersedes })
          : t("listing-history-first", { n, id });
        into.appendChild(text("div", line, "history-line"));
      });
    } catch (err) {
      into.appendChild(text("div", t("listing-history-failed", { detail: errText(err) })));
    }
  });
  // The button toggles. Two quick clicks are "open" then "close", and the
  // newest wins, so a load still in flight cannot reopen what was closed.
  let historyShown = false;
  historyBtn.onclick = () => {
    historyShown = !historyShown;
    void fillHistory(historyShown);
  };

  const withdrawBtn = text("button", t("listing-withdraw"), "button withdraw-listing") as HTMLButtonElement;
  withdrawBtn.disabled = row.status === "withdrawn";
  withdrawBtn.onclick = async () => {
    withdrawBtn.disabled = true;
    try {
      await call("listing.withdraw", { listing_id: row.listing_id });
      await reload();
    } catch (err) {
      withdrawBtn.disabled = false;
      wrap.appendChild(
        text("p", t("listing-withdraw-failed", { detail: errText(err) }), "row-error"),
      );
    }
  };

  wrap.append(historyBtn, withdrawBtn, historyHost);
  wrap.appendChild(publishToDirectory(row.listing_id));
  wrap.appendChild(availabilityEditor(row.listing_id));
  return wrap;
}

/// Journey step S7: the listing owner chooses a directory and publishes this
/// listing to it. Publishing is always the listing owner's own action -- a
/// directory never pulls. A refusal (over the limit, or a draft) is shown
/// with its reason.
function publishToDirectory(listingId: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "publish-to-directory";
  wrap.appendChild(text("h4", t("listing-publish-heading")));

  const didInput = input(t("listing-placeholder-directory-did"));
  didInput.className = "publish-directory-did";
  const btn = text("button", t("listing-publish-button"), "button publish-listing") as HTMLButtonElement;
  const status = text("p", "", "publish-status");

  btn.onclick = async () => {
    status.textContent = "";
    const source = didInput.value.trim();
    if (!source) return;
    btn.disabled = true;
    try {
      await call("directory.publish-to-source", { source, listing_id: listingId });
      status.textContent = t("listing-published");
    } catch (err) {
      status.textContent = t("listing-not-published", {
        reason: publishRefusalWords(err, errText(err)),
      });
    }
    btn.disabled = false;
  };

  wrap.append(didInput, btn, status);
  return wrap;
}

function buildEditor(onSaved: () => Promise<void>): { root: HTMLElement } {
  const root = document.createElement("div");
  root.className = "listing-editor";
  root.appendChild(text("h3", t("listing-editor-heading")));

  const slug = input(t("listing-placeholder-slug"));
  const title = input(t("listing-placeholder-title"));
  title.className = "listing-title-input";
  const summary = input(t("listing-placeholder-summary"));
  summary.className = "listing-summary-input";
  const categories = categoryPicker({
    className: "listing-categories-input",
    max: 8,
  });
  const address = input(t("listing-placeholder-address"));
  address.className = "listing-address-input";
  const status = select(codeOptions(STATUS_LABELS, ["active", "draft", "withdrawn"]), "active");

  root.append(
    field(t("listing-label-title"), title),
    field(t("listing-label-slug"), slug),
    field(t("listing-label-summary"), summary),
    field(t("listing-label-categories"), categories.root),
    field(t("listing-label-address"), address),
    field(t("listing-label-status"), status),
  );

  // --- booking ---
  const booking = block("booking", t("listing-block-booking"));
  const bookingMode = select(codeOptions(BOOKING_MODE_LABELS, ["enquiry", "slots", "order"]), "enquiry");
  const bookingLead = input(t("listing-placeholder-lead-time"), "0");
  const bookingCancel = input(t("listing-placeholder-cancellation-window"), "0");
  const bookingMax = input(t("listing-placeholder-max-per-booking"), "1");
  booking.body.append(
    field(t("listing-label-mode"), bookingMode),
    field(t("listing-label-lead-time"), bookingLead),
    field(t("listing-label-cancellation-window"), bookingCancel),
    field(t("listing-label-max-per-booking"), bookingMax),
  );

  // --- payment ---
  const payment = block("payment", t("listing-block-payment"));
  const payCurrency = input(t("listing-placeholder-currency"), "EUR");
  payCurrency.className = "payment-currency-input";
  const payModel = select(codeOptions(PRICING_MODEL_LABELS, ["per-hour", "fixed", "per-unit", "quote-only"]), "per-hour");
  const payAmount = input(t("listing-placeholder-amount"));
  payAmount.className = "payment-amount-input";
  const payFees = input(t("listing-placeholder-fees"));
  const payTax = checkbox(t("listing-label-tax-included"), false);
  const payMethodsWrap = document.createElement("div");
  payMethodsWrap.className = "payment-methods";
  const payMethodBoxes = PAYMENT_METHODS.map((m) => {
    const cb = checkbox(methodLabel(m), false);
    cb.box.value = m;
    cb.box.classList.add(`payment-method-${m}`);
    payMethodsWrap.appendChild(cb.wrap);
    return cb.box;
  });
  const payPayee = input(t("listing-placeholder-payee"));
  payment.body.append(
    field(t("listing-label-currency"), payCurrency),
    field(t("listing-label-model"), payModel),
    field(t("listing-label-amount"), payAmount),
    field(t("listing-label-fees"), payFees),
    payTax.wrap,
    field(t("listing-label-methods"), payMethodsWrap),
    field(t("listing-label-payee"), payPayee),
  );

  // --- product ---
  const product = block("product", t("listing-block-product"));
  const prodUnit = select(
    UNITS.map((u): [string, string] => [u, unitLabel(u)]),
    "piece",
  );
  const prodPack = input(t("listing-placeholder-pack-size"), "1");
  const prodCondition = select(codeOptions(CONDITION_LABELS, ["new", "used", "refurbished"]), "new");
  const prodSku = input(t("listing-placeholder-sku"));
  product.body.append(
    field(t("listing-label-unit"), prodUnit),
    field(t("listing-label-pack-size"), prodPack),
    field(t("listing-label-condition"), prodCondition),
    field(t("listing-label-sku"), prodSku),
  );

  // --- service ---
  const service = block("service", t("listing-block-service"));
  const svcDuration = input(t("listing-placeholder-duration"), "3600");
  const svcIncludes = input(t("listing-placeholder-includes"));
  const svcExcludes = input(t("listing-placeholder-excludes"));
  const svcPrereq = input(t("listing-placeholder-prerequisites"));
  service.body.append(
    field(t("listing-label-duration"), svcDuration),
    field(t("listing-label-includes"), svcIncludes),
    field(t("listing-label-excludes"), svcExcludes),
    field(t("listing-label-prerequisites"), svcPrereq),
  );

  // --- location ---
  const location = block("location", t("listing-block-location"));
  const locWhere = select(
    SERVICE_LOCATION_KINDS.map((kind): [string, string] => [kind, serviceLocationEditorLabel(kind)]),
    "remote",
  );
  const locDisclosure = select(codeOptions(DISCLOSURE_LABELS, ["on-agreement", "public"]), "on-agreement");
  const locArea = input(t("listing-placeholder-area"));
  location.body.append(
    field(t("listing-label-where"), locWhere),
    field(t("listing-label-disclosure"), locDisclosure),
    field(t("listing-label-service-area"), locArea),
  );

  // --- relationship ---
  const relationship = block("relationship", t("listing-block-relationship"));
  const relOpen = select(
    OPEN_TO_KINDS.map((kind): [string, string] => [kind, openToLabel(kind)]),
    "anyone",
  );
  const relMember = input(t("listing-placeholder-group-did"));
  relationship.body.append(field(t("listing-label-open-to"), relOpen), field(t("listing-label-group-did"), relMember));

  // --- service_record ---
  const serviceRecord = block("service_record", t("listing-block-service-record"));
  const srReceipt = checkbox(t("listing-label-fulfilment-receipt"), false);
  const srWarranty = input(t("listing-placeholder-warranty"), "0");
  const srRetention = input(t("listing-placeholder-retention"), "0");
  serviceRecord.body.append(
    srReceipt.wrap,
    field(t("listing-label-warranty"), srWarranty),
    field(t("listing-label-retention"), srRetention),
  );

  root.append(
    booking.details,
    payment.details,
    product.details,
    service.details,
    location.details,
    relationship.details,
    serviceRecord.details,
  );

  const saveBtn = text("button", t("listing-save-button"), "button save-listing") as HTMLButtonElement;
  const result = text("p", "", "listing-save-result");
  root.append(saveBtn, result);

  saveBtn.onclick = async () => {
    result.textContent = "";
    const form: ListingForm = {
      slug: slug.value,
      title: title.value,
      summary: summary.value,
      categories: categories.codes(),
      conversation_address: address.value,
      status: status.value as ListingForm["status"],
      booking: {
        enabled: booking.enabled.checked,
        value: {
          mode: bookingMode.value as "slots" | "order" | "enquiry",
          lead_time_secs: bookingLead.value,
          cancellation_window_secs: bookingCancel.value,
          max_per_booking: bookingMax.value,
        },
      },
      payment: {
        enabled: payment.enabled.checked,
        value: {
          currency: payCurrency.value,
          model: payModel.value as "fixed" | "per-hour" | "per-unit" | "quote-only",
          amount: payAmount.value,
          tax_included: payTax.box.checked,
          fees: payFees.value,
          methods: payMethodBoxes.filter((b) => b.checked).map((b) => b.value),
          payee: payPayee.value,
        },
      },
      product: {
        enabled: product.enabled.checked,
        value: {
          unit: prodUnit.value,
          pack_size: prodPack.value,
          condition: prodCondition.value as "new" | "used" | "refurbished",
          sku: prodSku.value,
        },
      },
      service: {
        enabled: service.enabled.checked,
        value: {
          duration_secs: svcDuration.value,
          includes: svcIncludes.value,
          excludes: svcExcludes.value,
          prerequisites: svcPrereq.value,
        },
      },
      location: {
        enabled: location.enabled.checked,
        value: {
          where: locWhere.value as ServiceLocationKind,
          address_disclosure: locDisclosure.value as "on-agreement" | "public",
          area_label: locArea.value,
        },
      },
      relationship: {
        enabled: relationship.enabled.checked,
        value: {
          open_to: relOpen.value as OpenTo,
          member_of: relMember.value,
        },
      },
      service_record: {
        enabled: serviceRecord.enabled.checked,
        value: {
          issues_fulfilment_receipt: srReceipt.box.checked,
          warranty_secs: srWarranty.value,
          retention_secs: srRetention.value,
        },
      },
    };

    let params: Record<string, unknown>;
    try {
      params = buildSetListingParams(form);
    } catch (err) {
      result.textContent = errText(err);
      return;
    }
    saveBtn.disabled = true;
    try {
      const res = await call<{ listing_id: string; record_id: string; version_count: number }>(
        "listing.set",
        params,
      );
      result.textContent = t("listing-saved", {
        id: res.listing_id,
        version: String(res.version_count),
        record: res.record_id,
      });
      await onSaved();
    } catch (err) {
      result.textContent = t("common-save-failed", { detail: errText(err) });
    }
    saveBtn.disabled = false;
  };

  return { root };
}

function availabilityEditor(listingId: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "availability-editor";
  wrap.appendChild(text("h4", t("listing-availability-heading")));

  const slotsHost = document.createElement("div");
  slotsHost.className = "slot-list";
  wrap.appendChild(slotsHost);

  const start = document.createElement("input");
  start.type = "datetime-local";
  const end = document.createElement("input");
  end.type = "datetime-local";
  const capacity = input(t("listing-placeholder-capacity"), "1");
  const addBtn = text("button", t("listing-add-slot"), "button add-slot") as HTMLButtonElement;
  const err = text("p", "", "slot-error");

  const reloadSlots = refiller(slotsHost, async (into) => {
    try {
      const res = await call<{
        slots: Array<{ slot_id: string; start_secs: number; end_secs: number; capacity: number }>;
      }>("availability.list", { listing_id: listingId });
      for (const s of res.slots) {
        const line = document.createElement("div");
        line.className = "slot-line";
        const startStr = formatDateTime(s.start_secs);
        const endStr = formatDateTime(s.end_secs);
        line.appendChild(
          text("span", t("listing-slot-line", { start: startStr, end: endStr, capacity: s.capacity })),
        );
        const rm = text("button", t("common-remove"), "button remove-slot") as HTMLButtonElement;
        rm.onclick = async () => {
          await call("availability.remove", { slot_id: s.slot_id });
          await reloadSlots();
        };
        line.appendChild(rm);
        into.appendChild(line);
      }
    } catch (e) {
      into.appendChild(text("p", t("listing-slots-load-failed", { detail: errText(e) })));
    }
  });

  addBtn.onclick = async () => {
    err.textContent = "";
    try {
      const slot = slotFromLocalDatetimes(start.value, end.value, capacity.value);
      await call("availability.set", { listing_id: listingId, slots: [slot] });
      await reloadSlots();
    } catch (e) {
      err.textContent = errText(e);
    }
  };

  wrap.append(
    field(t("listing-label-start"), start),
    field(t("listing-label-end"), end),
    field(t("listing-label-capacity"), capacity),
    addBtn,
    err,
  );
  void reloadSlots();
  return wrap;
}

export function verifyParams(pasted: string): { envelope: string } {
  return { envelope: pasted.trim() };
}

function buildVerifyBox(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "listing-verify";
  wrap.appendChild(text("h3", t("listing-verify-heading")));

  const area = document.createElement("textarea");
  area.placeholder = t("listing-verify-placeholder");
  area.className = "verify-input";
  const btn = text("button", t("listing-verify-button"), "button verify-listing") as HTMLButtonElement;
  const out = document.createElement("div");
  out.className = "verify-result";

  btn.onclick = async () => {
    out.replaceChildren();
    try {
      JSON.parse(area.value);
    } catch {
      out.appendChild(text("p", t("listing-verify-bad-json")));
      return;
    }
    try {
      const res = await call<{
        verified: boolean;
        reason?: string;
        issuer?: string;
        conversation_address?: string;
        status?: string;
      }>("listing.verify", verifyParams(area.value));
      if (res.verified) {
        out.appendChild(text("p", t("listing-verified"), "verify-ok"));
        out.appendChild(text("div", t("listing-verify-issuer", { issuer: res.issuer ?? "" })));
        out.appendChild(
          text("div", t("listing-verify-address", { address: res.conversation_address ?? "" })),
        );
        out.appendChild(
          text("div", t("listing-verify-status", { status: codeWord(STATUS_LABELS, res.status ?? "") })),
        );
      } else {
        out.appendChild(text("p", t("listing-not-verified"), "verify-bad"));
        out.appendChild(text("div", res.reason ?? ""));
      }
    } catch (err) {
      out.appendChild(text("p", t("listing-check-failed", { detail: errText(err) })));
    }
  };

  wrap.append(area, btn, out);
  return wrap;
}
