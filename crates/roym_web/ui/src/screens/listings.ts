import {
  buildSetListingParams,
  ListingInputError,
  slotFromLocalDatetimes,
  type ListingForm,
} from "../listings/editor";
import { call, RpcError } from "../rpc";

interface ListingListRow {
  listing_id: string;
  slug: string;
  status: string;
  record_id: string;
  updated_at_secs: number;
  version_count: number;
}

function errText(err: unknown): string {
  if (err instanceof RpcError || err instanceof ListingInputError) return err.message;
  return err instanceof Error ? err.message : String(err);
}

/// A stranger's listing text is only ever a text node.
function text(tag: string, value: string, className?: string): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = value;
  if (className) el.className = className;
  return el;
}

function labelled(labelText: string, control: HTMLElement): HTMLElement {
  const label = document.createElement("label");
  label.className = "field";
  label.appendChild(text("span", labelText));
  label.appendChild(control);
  return label;
}

function input(placeholder: string, value = ""): HTMLInputElement {
  const el = document.createElement("input");
  el.placeholder = placeholder;
  el.value = value;
  return el;
}

function select(options: string[], value?: string): HTMLSelectElement {
  const el = document.createElement("select");
  for (const opt of options) {
    const o = document.createElement("option");
    o.value = opt;
    o.textContent = opt;
    el.appendChild(o);
  }
  if (value) el.value = value;
  return el;
}

function checkbox(labelText: string, checked = false): { wrap: HTMLElement; box: HTMLInputElement } {
  const wrap = document.createElement("label");
  wrap.className = "checkbox-field";
  const box = document.createElement("input");
  box.type = "checkbox";
  box.checked = checked;
  wrap.append(box, text("span", labelText));
  return { wrap, box };
}

/// A collapsed `<details>` block with an "included" checkbox in the summary.
function block(name: string): { details: HTMLDetailsElement; enabled: HTMLInputElement; body: HTMLElement } {
  const details = document.createElement("details");
  details.className = `listing-block block-${name}`;
  const summary = document.createElement("summary");
  const enabled = document.createElement("input");
  enabled.type = "checkbox";
  enabled.className = "block-enabled";
  enabled.onclick = (e) => e.stopPropagation();
  summary.append(enabled, text("span", name));
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
  box.appendChild(text("h2", "Listings"));

  const listHost = document.createElement("div");
  listHost.className = "listing-list";
  box.appendChild(listHost);

  const editor = buildEditor(() => reload());
  box.appendChild(editor.root);

  box.appendChild(buildVerifyBox());

  async function reload() {
    listHost.replaceChildren();
    listHost.appendChild(text("h3", "Your listings"));
    let rows: ListingListRow[] = [];
    try {
      const res = await call<{ listings: ListingListRow[] }>("listing.list");
      rows = res.listings;
    } catch (err) {
      listHost.appendChild(text("p", `Could not load listings: ${errText(err)}`));
      return;
    }
    if (rows.length === 0) {
      listHost.appendChild(text("p", "No listings yet."));
    }
    for (const row of rows) {
      listHost.appendChild(listingRow(row, reload));
    }
  }

  await reload();
  container.appendChild(box);
}

function listingRow(row: ListingListRow, reload: () => Promise<void>): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "listing-row";
  wrap.dataset.status = row.status;

  wrap.appendChild(text("span", row.slug, "listing-slug"));
  wrap.appendChild(text("span", row.status, `listing-status status-${row.status}`));
  wrap.appendChild(text("span", `v${row.version_count}`, "listing-version"));

  const historyBtn = text("button", "History", "button listing-history-btn") as HTMLButtonElement;
  const historyHost = document.createElement("div");
  historyHost.className = "listing-history";
  historyBtn.onclick = async () => {
    if (historyHost.childElementCount > 0) {
      historyHost.replaceChildren();
      return;
    }
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
        const line =
          `#${i + 1} ${env.record_id ?? ""}` +
          (env.supersedes ? ` (supersedes ${env.supersedes})` : " (first version)");
        historyHost.appendChild(text("div", line, "history-line"));
      });
    } catch (err) {
      historyHost.appendChild(text("div", `History failed: ${errText(err)}`));
    }
  };

  const withdrawBtn = text("button", "Withdraw", "button withdraw-listing") as HTMLButtonElement;
  withdrawBtn.disabled = row.status === "withdrawn";
  withdrawBtn.onclick = async () => {
    withdrawBtn.disabled = true;
    try {
      await call("listing.withdraw", { listing_id: row.listing_id });
      await reload();
    } catch (err) {
      withdrawBtn.disabled = false;
      wrap.appendChild(text("p", `Withdraw failed: ${errText(err)}`, "row-error"));
    }
  };

  wrap.append(historyBtn, withdrawBtn, historyHost);
  wrap.appendChild(availabilityEditor(row.listing_id));
  return wrap;
}

function buildEditor(onSaved: () => Promise<void>): { root: HTMLElement } {
  const root = document.createElement("div");
  root.className = "listing-editor";
  root.appendChild(text("h3", "New / edit a listing"));

  const slug = input("slug (optional; derived from the title)");
  const title = input("title");
  title.className = "listing-title-input";
  const summary = input("summary");
  const categories = input("categories, comma separated");
  const address = input("conversation address (optional; taken from your profile)");
  const status = select(["active", "draft", "withdrawn"], "active");

  root.append(
    labelled("Title", title),
    labelled("Slug", slug),
    labelled("Summary", summary),
    labelled("Categories", categories),
    labelled("Conversation address", address),
    labelled("Status", status),
  );

  // --- booking ---
  const booking = block("booking");
  const bookingMode = select(["enquiry", "slots", "order"], "enquiry");
  const bookingLead = input("lead time (seconds)", "0");
  const bookingCancel = input("cancellation window (seconds)", "0");
  const bookingMax = input("max per booking", "1");
  booking.body.append(
    labelled("Mode", bookingMode),
    labelled("Lead time (s)", bookingLead),
    labelled("Cancellation window (s)", bookingCancel),
    labelled("Max per booking", bookingMax),
  );

  // --- payment ---
  const payment = block("payment");
  const payCurrency = input("currency (ISO-4217, e.g. EUR)", "EUR");
  const payModel = select(["per-hour", "fixed", "per-unit", "quote-only"], "per-hour");
  const payAmount = input("amount, e.g. 35.50");
  payAmount.className = "payment-amount-input";
  const payFees = input("fees, e.g. 2.50 (optional)");
  const payTax = checkbox("tax included", false);
  const payMethods = input("methods, comma separated");
  const payPayee = input("payee (informational only)");
  payment.body.append(
    labelled("Currency", payCurrency),
    labelled("Model", payModel),
    labelled("Amount", payAmount),
    labelled("Fees", payFees),
    payTax.wrap,
    labelled("Methods", payMethods),
    labelled("Payee", payPayee),
  );

  // --- product ---
  const product = block("product");
  const prodUnit = input("unit, e.g. item", "item");
  const prodPack = input("pack size", "1");
  const prodCondition = select(["new", "used", "refurbished"], "new");
  const prodSku = input("SKU (optional)");
  product.body.append(
    labelled("Unit", prodUnit),
    labelled("Pack size", prodPack),
    labelled("Condition", prodCondition),
    labelled("SKU", prodSku),
  );

  // --- service ---
  const service = block("service");
  const svcDuration = input("duration (seconds)", "3600");
  const svcIncludes = input("includes, comma separated");
  const svcExcludes = input("excludes, comma separated");
  const svcPrereq = input("prerequisites, comma separated");
  service.body.append(
    labelled("Duration (s)", svcDuration),
    labelled("Includes", svcIncludes),
    labelled("Excludes", svcExcludes),
    labelled("Prerequisites", svcPrereq),
  );

  // --- location ---
  const location = block("location");
  const locWhere = select(["remote", "at-provider", "at-customer"], "remote");
  const locDisclosure = select(["on-agreement", "public"], "on-agreement");
  const locArea = input("service area label (optional)");
  location.body.append(
    labelled("Where", locWhere),
    labelled("Address disclosure", locDisclosure),
    labelled("Service area", locArea),
  );

  // --- relationship ---
  const relationship = block("relationship");
  const relOpen = select(["anyone", "members", "referral", "existing-customers"], "anyone");
  const relMember = input("group DID (required for members)");
  relationship.body.append(labelled("Open to", relOpen), labelled("Group DID", relMember));

  // --- service_record ---
  const serviceRecord = block("service_record");
  const srReceipt = checkbox("issues a fulfilment receipt", false);
  const srWarranty = input("warranty (seconds)", "0");
  const srRetention = input("retention (seconds)", "0");
  serviceRecord.body.append(
    srReceipt.wrap,
    labelled("Warranty (s)", srWarranty),
    labelled("Retention (s)", srRetention),
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

  const saveBtn = text("button", "Save listing", "button save-listing") as HTMLButtonElement;
  const result = text("p", "", "listing-save-result");
  root.append(saveBtn, result);

  saveBtn.onclick = async () => {
    result.textContent = "";
    const form: ListingForm = {
      slug: slug.value,
      title: title.value,
      summary: summary.value,
      categories: categories.value,
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
          methods: payMethods.value,
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
          where: locWhere.value as "at-provider" | "at-customer" | "remote",
          address_disclosure: locDisclosure.value as "on-agreement" | "public",
          area_label: locArea.value,
        },
      },
      relationship: {
        enabled: relationship.enabled.checked,
        value: {
          open_to: relOpen.value as "anyone" | "members" | "referral" | "existing-customers",
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
      result.textContent = `Saved ${res.listing_id} as version ${res.version_count} (${res.record_id})`;
      await onSaved();
    } catch (err) {
      result.textContent = `Save failed: ${errText(err)}`;
    }
    saveBtn.disabled = false;
  };

  return { root };
}

function availabilityEditor(listingId: string): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "availability-editor";
  wrap.appendChild(text("h4", "Availability (for slot bookings)"));

  const slotsHost = document.createElement("div");
  slotsHost.className = "slot-list";
  wrap.appendChild(slotsHost);

  const start = document.createElement("input");
  start.type = "datetime-local";
  const end = document.createElement("input");
  end.type = "datetime-local";
  const capacity = input("capacity", "1");
  const addBtn = text("button", "Add slot", "button add-slot") as HTMLButtonElement;
  const err = text("p", "", "slot-error");

  async function reloadSlots() {
    slotsHost.replaceChildren();
    try {
      const res = await call<{
        slots: Array<{ slot_id: string; start_secs: number; end_secs: number; capacity: number }>;
      }>("availability.list", { listing_id: listingId });
      for (const s of res.slots) {
        const line = document.createElement("div");
        line.className = "slot-line";
        const startStr = new Date(s.start_secs * 1000).toLocaleString();
        const endStr = new Date(s.end_secs * 1000).toLocaleString();
        line.appendChild(text("span", `${startStr} - ${endStr} (x${s.capacity})`));
        const rm = text("button", "Remove", "button remove-slot") as HTMLButtonElement;
        rm.onclick = async () => {
          await call("availability.remove", { slot_id: s.slot_id });
          await reloadSlots();
        };
        line.appendChild(rm);
        slotsHost.appendChild(line);
      }
    } catch (e) {
      slotsHost.appendChild(text("p", `Could not load slots: ${errText(e)}`));
    }
  }

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
    labelled("Start", start),
    labelled("End", end),
    labelled("Capacity", capacity),
    addBtn,
    err,
  );
  void reloadSlots();
  return wrap;
}

function buildVerifyBox(): HTMLElement {
  const wrap = document.createElement("div");
  wrap.className = "listing-verify";
  wrap.appendChild(text("h3", "Check a listing someone sent me"));

  const area = document.createElement("textarea");
  area.placeholder = "Paste a listing envelope JSON";
  area.className = "verify-input";
  const btn = text("button", "Check", "button verify-listing") as HTMLButtonElement;
  const out = document.createElement("div");
  out.className = "verify-result";

  btn.onclick = async () => {
    out.replaceChildren();
    let envelope: unknown;
    try {
      envelope = JSON.parse(area.value);
    } catch {
      out.appendChild(text("p", "That is not valid JSON."));
      return;
    }
    try {
      const res = await call<{
        verified: boolean;
        reason?: string;
        issuer?: string;
        conversation_address?: string;
        status?: string;
      }>("listing.verify", { envelope });
      if (res.verified) {
        out.appendChild(text("p", "Verified.", "verify-ok"));
        out.appendChild(text("div", `Issuer: ${res.issuer ?? ""}`));
        out.appendChild(text("div", `Conversation address: ${res.conversation_address ?? ""}`));
        out.appendChild(text("div", `Status: ${res.status ?? ""}`));
      } else {
        out.appendChild(text("p", "Not verified -- treat as unknown.", "verify-bad"));
        out.appendChild(text("div", res.reason ?? ""));
      }
    } catch (err) {
      out.appendChild(text("p", `Check failed: ${errText(err)}`));
    }
  };

  wrap.append(area, btn, out);
  return wrap;
}
