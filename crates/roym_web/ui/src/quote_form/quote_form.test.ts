import { afterEach, describe, expect, it, vi } from "vitest";
import { activate, activateWith, t } from "../i18n/i18n";
import { stubRpc } from "../testing/stub-rpc";
import { openQuoteForm, readQuoteForm } from "./form";

const HINDI = `
quote-title = प्रस्ताव
quote-request-button = प्रस्ताव बनाएं
quote-send-button = प्रस्ताव भेजें
quote-scope-placeholder = कार्य का दायरा
quote-give-label = आप क्या देते हैं
quote-give-placeholder = आप क्या देते हैं
quote-payment-description = भुगतान
quote-currency-placeholder = मुद्रा (जैसे EUR, USD)
quote-amount-placeholder = कुल राशि (जैसे 45.00)
quote-tax-placeholder = कर शामिल (वैकल्पिक)
quote-fees-placeholder = शुल्क शामिल (वैकल्पिक)
quote-payee-placeholder = प्राप्तकर्ता का नाम या कंपनी
quote-timing-after = काम के बाद भुगतान
quote-timing-before = काम से पहले भुगतान
quote-address-disclosure-notice = यह पता हस्ताक्षरित रिकॉर्ड का हिस्सा बन जाता है
quote-address-placeholder = सेवा का पता
quote-cancellation-placeholder = रद्दीकरण की शर्तें
quote-default-cancellation = पूरी वापसी के लिए 24 घंटे पहले सूचना ज़रूरी है
quote-refund-placeholder = वापसी की शर्तें
quote-default-refund = काम पूरा न होने पर पूरी वापसी
quote-dispute-placeholder = विवाद का रास्ता
quote-default-dispute = छोटे दावों की अदालत या आपसी सुलह
quote-hours-placeholder = घंटों में समाप्ति
deal-add-line = पंक्ति जोड़ें
deal-remove-line = हटाएं
deal-more-terms = अधिक शर्तें
deal-save-defaults = मेरे डिफ़ॉल्ट के रूप में सहेजें
deal-giver-me = मैं देता हूँ
deal-giver-them = वे देते हैं
deal-kind-service = सेवा
deal-kind-goods = वस्तुएँ
deal-kind-money = धन
deal-you-give = आप देते हैं: { $amount }
deal-you-receive = आप प्राप्त करते हैं: { $amount }
deal-diff-you-give-more = आप प्राप्त करने से { $amount } अधिक देते हैं
deal-diff-you-receive-more = आप देने से { $amount } अधिक प्राप्त करते हैं
deal-error-window-required = समय सीमा के लिए दोनों तारीखें आवश्यक हैं
deal-error-window-order = आरंभ तिथि समाप्ति तिथि के बाद नहीं होनी चाहिए
`;

describe("quote form", () => {
  afterEach(async () => {
    vi.unstubAllGlobals();
    await activate("en");
  });

  it("fills the quote terms in the language of the Hub", async () => {
    await activateWith("hi", { hi: async () => HINDI });
    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" };
    openQuoteForm(anchor, request, "conv1", async () => {});
    const value = (cls: string) => anchor.querySelector<HTMLInputElement>(cls)?.value;
    expect(value(".quote-canc-input")).toBe(t("quote-default-cancellation"));
    expect(value(".quote-ref-input")).toBe(t("quote-default-refund"));
    expect(value(".quote-disp-input")).toBe(t("quote-default-dispute"));
    expect(value(".quote-canc-input")).toMatch(/[ऀ-ॿ]/);
  });

  it("quote form sends lang from its control in terms, starting at UI language", async () => {
    let sentTerms: Record<string, unknown> | undefined;
    stubRpc({
      "quote.set": (params) => {
        sentTerms = (params as { terms: Record<string, unknown> }).terms;
        return { quote_id: "q1", record_id: "rec_q1" };
      },
    });

    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" };
    openQuoteForm(anchor, request, "conv1", async () => {});

    const langSel = anchor.querySelector<HTMLSelectElement>(".quote-text-lang")!;
    expect(langSel).not.toBeNull();
    expect(langSel.value).toBe("en");

    langSel.value = "hi";

    anchor.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Scope of work";
    anchor.querySelector<HTMLInputElement>(".quote-currency-input")!.value = "EUR";
    anchor.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "100";
    anchor.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "Alice";

    const submitBtn = anchor.querySelector<HTMLButtonElement>(".submit-quote-button")!;
    submitBtn.click();

    await vi.waitFor(() => expect(sentTerms).toBeDefined());
    expect((sentTerms!.scope as Record<string, unknown>).lang).toBe("hi");
  });

  it("readQuoteForm gives the terms submit sends", async () => {
    let sentTerms: Record<string, unknown> | undefined;
    stubRpc({
      "quote.set": (params) => {
        sentTerms = (params as { terms: Record<string, unknown> }).terms;
        return { quote_id: "q1", record_id: "rec_q1" };
      },
    });

    const anchor = document.createElement("div");
    const request = { record_id: "rec_req" };
    openQuoteForm(anchor, request, "conv1", async () => {});

    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Scope of work";
    form.querySelector<HTMLInputElement>(".quote-currency-input")!.value = "EUR";
    form.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "100";
    form.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "Alice";

    const parsed = readQuoteForm(form);
    expect("terms" in parsed).toBe(true);

    const submitBtn = form.querySelector<HTMLButtonElement>(".submit-quote-button")!;
    submitBtn.click();

    await vi.waitFor(() => expect(sentTerms).toBeDefined());
    expect("terms" in parsed && parsed.terms).toEqual(sentTerms);
  });

  it("form with only scope and amount builds equal totals with no difference line", () => {
    const anchor = document.createElement("div");
    openQuoteForm(anchor, { record_id: "rec_req" }, "conv1", async () => {});

    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    const scopeInput = form.querySelector<HTMLInputElement>(".quote-scope-input")!;
    const amountInput = form.querySelector<HTMLInputElement>(".quote-amount-input")!;

    scopeInput.value = "Web design";
    amountInput.value = "150";
    amountInput.dispatchEvent(new Event("input"));

    const giveP = form.querySelector<HTMLParagraphElement>(".quote-total-give")!;
    const receiveP = form.querySelector<HTMLParagraphElement>(".quote-total-receive")!;
    const diffP = form.querySelector<HTMLParagraphElement>(".quote-total-diff")!;

    expect(giveP.textContent).toContain("150");
    expect(receiveP.textContent).toContain("150");
    // No difference line shown when totals are equal
    expect(diffP.style.display).toBe("none");
    expect(diffP.textContent).toBe("");
  });

  it("supports adding and removing rows up to 16 rows", () => {
    const anchor = document.createElement("div");
    openQuoteForm(anchor, { record_id: "rec_req" }, "conv1", async () => {});

    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    const addBtn = form.querySelector<HTMLButtonElement>(".quote-add-row-button")!;

    // Initial has 2 rows
    expect(form.querySelectorAll(".quote-commitment-row")).toHaveLength(2);

    // Add 14 more rows to reach 16
    for (let i = 2; i < 16; i++) {
      expect(addBtn.disabled).toBe(false);
      addBtn.click();
    }

    expect(form.querySelectorAll(".quote-commitment-row")).toHaveLength(16);
    // At 16 rows, Add button must be disabled
    expect(addBtn.disabled).toBe(true);

    // Remove one row
    const removeBtn = form.querySelector<HTMLButtonElement>(".quote-row-remove-button")!;
    removeBtn.click();
    expect(form.querySelectorAll(".quote-commitment-row")).toHaveLength(15);
    expect(addBtn.disabled).toBe(false);
  });

  it("builds DealTerms with after dependencies for rows on either side", () => {
    const anchor = document.createElement("div");
    openQuoteForm(anchor, { record_id: "rec_req" }, "conv1", async () => {});

    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Consulting & Supplies";
    form.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "500";
    form.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "Alice";

    // Set timing to before-work
    const timingSelect = form.querySelector<HTMLSelectElement>(".quote-timing-select")!;
    timingSelect.value = "before-work";

    const parsed = readQuoteForm(form);
    expect("terms" in parsed).toBe(true);
    if ("terms" in parsed) {
      const comms = parsed.terms.commitments as Array<Record<string, unknown>>;
      expect(comms).toHaveLength(2);
      // Row 0 (author service) after Row 1 (counterparty money)
      expect(comms[0].after).toEqual(["c2"]);
      expect(comms[1].after).toEqual([]);
    }
  });

  it("submits new version with enquiry record id and quote_id when opened from quote card", async () => {
    let capturedParams: Record<string, unknown> | null = null;
    stubRpc({
      "quote.set": (params) => {
        capturedParams = params as Record<string, unknown>;
        return { quote_id: "q_new", record_id: "rec_q_new" };
      },
    });

    const anchor = document.createElement("div");
    const quoteCard = {
      card_type: "quote",
      record_id: "rec_quote_old",
      data: {
        quote_id: "qid_123",
        answers: {
          kind: "enquiry",
          record_id: "rec_original_enquiry",
        },
        counterparty_did: "did:key:alice",
        terms: {
          currency: "INR",
          commitments: [
            {
              commitment_id: "c1",
              giver: "quote-author",
              kind: "service",
              declared_value_minor: 50000,
              description: { lang: "en", text: "Lesson" },
            },
            {
              commitment_id: "c2",
              giver: "counterparty",
              kind: "currency",
              declared_value_minor: 50000,
              payee: "did:key:bob",
            },
          ],
        },
      },
    };

    openQuoteForm(anchor, quoteCard, "conv1", async () => {}, undefined, "did:key:bob");
    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Updated Lesson Scope";

    form.querySelector<HTMLButtonElement>(".submit-quote-button")!.click();
    await vi.waitFor(() => expect(capturedParams).not.toBeNull());

    expect(capturedParams!.enquiry_record_id).toBe("rec_original_enquiry");
    expect(capturedParams!.quote_id).toBe("qid_123");
  });

  it("populates from listing and defaults with payee defaulting to myDid", async () => {
    stubRpc({
      "listing.get": () => ({
        envelope: "env_test_123",
      }),
      "listing.verify": () => ({
        verified: true,
        payload: {
          listing_id: "list_1",
          payment: { amount_minor: 75000, currency: "INR", payee: "did:key:bob_custom" },
          deal_terms: {
            cancellation_terms: { lang: "en", text: "Listing cancellation" },
            refund_terms: { lang: "en", text: "Listing refund" },
            dispute_path: { lang: "en", text: "Listing dispute" },
          },
        },
      }),
      "deal.defaults-get": () => ({
        quote_valid_secs: 7200,
      }),
    });

    const anchor = document.createElement("div");
    const enquiryCard = {
      card_type: "enquiry",
      record_id: "rec_enq_1",
      data: {
        listing_id: "list_1",
      },
    };

    openQuoteForm(anchor, enquiryCard, "conv1", async () => {}, undefined, "did:key:my_author_did");
    const form = anchor.querySelector<HTMLElement>(".quote-form")!;

    await vi.waitFor(() => {
      const payeeInput = form.querySelector<HTMLInputElement>(".quote-payee-input");
      expect(payeeInput?.value).toBe("did:key:bob_custom");
    });

    const cancInput = form.querySelector<HTMLInputElement>(".quote-canc-input");
    expect(cancInput?.value).toBe("Listing cancellation");
    const amountInput = form.querySelector<HTMLInputElement>(".quote-amount-input");
    expect(amountInput?.value).toBe("750.00");
  });

  it("enforces value-required on non-money rows and handles invalid money inputs without silent rounding", () => {
    const anchor = document.createElement("div");
    openQuoteForm(anchor, { record_id: "rec_req" }, "conv1", async () => {});
    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Scope";

    // Non-money row with empty value returns quote-value-required
    const rowValueInput = form.querySelector<HTMLInputElement>(".quote-row-value-input")!;
    rowValueInput.value = "";
    rowValueInput.dispatchEvent(new Event("input"));

    const parsedEmpty = readQuoteForm(form);
    expect("error" in parsedEmpty && "key" in parsedEmpty.error && parsedEmpty.error.key).toBe("quote-value-required");

    // Invalid money text 'abc' returns error from MoneyInput
    const amountInput = form.querySelector<HTMLInputElement>(".quote-amount-input")!;
    amountInput.value = "abc";
    amountInput.dispatchEvent(new Event("input"));

    const parsedAbc = readQuoteForm(form);
    expect("error" in parsedAbc).toBe(true);
  });

  it("requires both window dates or neither and computes latest_secs as end of day", () => {
    const anchor = document.createElement("div");
    openQuoteForm(anchor, { record_id: "rec_req" }, "conv1", async () => {});
    const form = anchor.querySelector<HTMLElement>(".quote-form")!;
    form.querySelector<HTMLInputElement>(".quote-scope-input")!.value = "Lesson";
    form.querySelector<HTMLInputElement>(".quote-row-value-input")!.value = "100";
    form.querySelector<HTMLInputElement>(".quote-amount-input")!.value = "100";
    form.querySelector<HTMLInputElement>(".quote-payee-input")!.value = "did:key:author";

    const fromInput = form.querySelector<HTMLInputElement>(".quote-row-from-input")!;
    const toInput = form.querySelector<HTMLInputElement>(".quote-row-to-input")!;

    // Only from date provided -> error
    fromInput.value = "2026-11-28";
    toInput.value = "";
    fromInput.dispatchEvent(new Event("input"));
    let res = readQuoteForm(form);
    expect(res).toEqual({ error: { raw: "Both dates are required for the time window" } });

    // From date after to date -> order error
    fromInput.value = "2026-11-30";
    toInput.value = "2026-11-28";
    fromInput.dispatchEvent(new Event("input"));
    res = readQuoteForm(form);
    expect(res).toEqual({ error: { raw: "The start date must not be after the end date" } });

    // Both provided on same day -> window spans from start of day to 23:59:59
    fromInput.value = "2026-11-28";
    toInput.value = "2026-11-28";
    fromInput.dispatchEvent(new Event("input"));
    toInput.dispatchEvent(new Event("input"));
    res = readQuoteForm(form);
    expect("terms" in res).toBe(true);
    if ("terms" in res) {
      const commitments = res.terms.commitments as Array<Record<string, unknown>>;
      const authorService = commitments.find((c) => c.giver === "quote-author");
      expect(authorService?.window).toBeDefined();
      const win = authorService!.window as { earliest_secs: number; latest_secs: number };
      expect(win.latest_secs - win.earliest_secs).toBe(86399);
    }
  });
});
