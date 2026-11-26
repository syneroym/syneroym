import { test, expect, type Page } from '../fixtures';
import { loginWithDelegatedKey } from '../hub-helpers';
import { readE2EPorts } from '../ports';

let HUB_URL: string;

test.describe('Roym deal screens', () => {
  test.beforeAll(() => {
    const ports = readE2EPorts();
    HUB_URL = process.env.ROYM_HUB_URL || `http://127.0.0.1:${ports.gatewayPort}`;
  });

  test('1. quote card shows both totals and difference and counterparty can agree', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'deal-totals-test-host';
      document.body.appendChild(host);

      const cardEl = win.RoymRegistry.renderCard({
        type: 'quote',
        version: 1,
        data: {
          record_id: 'rec_quote_three',
          role: 'counterparty',
          terms: {
            currency: 'INR',
            commitments: [
              {
                commitment_id: 'c1',
                giver: 'quote-author',
                resource_kind: 'service',
                description: { lang: 'en', text: 'Woodwork lesson' },
                declared_value_minor: 50000,
              },
              {
                commitment_id: 'c2',
                giver: 'counterparty',
                resource_kind: 'goods',
                description: { lang: 'en', text: 'Chisels set' },
                declared_value_minor: 30000,
              },
              {
                commitment_id: 'c3',
                giver: 'counterparty',
                resource_kind: 'currency',
                declared_value_minor: 10000,
                payee: 'did:key:author',
              },
            ],
          },
        },
      });
      host.appendChild(cardEl);

      const actionsWrap = document.createElement('div');
      actionsWrap.id = 'deal-actions-wrap';
      host.appendChild(actionsWrap);
      win.RoymMessages.renderCardActions(
        actionsWrap,
        {
          message_id: 'msg_quote_three',
          conversation: 'conv_deal',
          direction: 'incoming',
          card_type: 'quote',
          version: 1,
          record_id: 'rec_quote_three',
          stored_at_secs: 1000,
          data: {
            counterparty_did: 'did:key:author',
            terms: { currency: 'INR' },
          },
        },
        'conv_deal',
        'did:key:viewer',
        async () => {},
      );
    });

    const host = page.locator('#deal-totals-test-host');
    await expect(host.locator('.quote-total-give')).toContainText('₹400.00');
    await expect(host.locator('.quote-total-receive')).toContainText('₹500.00');
    await expect(host.locator('.quote-total-diff')).toContainText('₹100.00 more than you give');

    let acceptPayload: any = null;
    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'agreement.accept') {
        acceptPayload = body.params;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: { agreement_record_id: 'rec_agr_1' } }),
        });
        return;
      }
      await route.fallback();
    });

    await host.locator('.accept-quote-button').click();
    await expect.poll(() => acceptPayload).toEqual(expect.objectContaining({ quote_record_id: 'rec_quote_three' }));
  });

  test('2. closing actions leave out future commitments until selected and signed', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    const nextWeekSecs = Math.floor(Date.now() / 1000) + 7 * 86400;
    await page.evaluate((futureSecs) => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'deal-close-test-host';
      document.body.appendChild(host);

      const agreementView = {
        quote_record_id: 'rec_agr_close',
        quote_author_did: 'did:key:author',
        state: 'agreed',
        terms: {
          currency: 'INR',
          commitments: [
            {
              commitment_id: 'c_goods',
              giver: 'quote-author',
              resource_kind: 'goods',
              description: { lang: 'en', text: 'Wood planks' },
              declared_value_minor: 20000,
            },
            {
              commitment_id: 'c_lesson',
              giver: 'quote-author',
              resource_kind: 'service',
              description: { lang: 'en', text: 'Future lesson' },
              declared_value_minor: 30000,
              window: {
                earliest_secs: futureSecs,
                latest_secs: futureSecs + 3600,
              },
            },
          ],
        },
        commitments: [
          { commitment_id: 'c_goods', state: 'promised' },
          { commitment_id: 'c_lesson', state: 'promised' },
        ],
      };

      const el = win.RoymDeals.renderAgreementPanelView(
        agreementView,
        'did:key:counterparty',
        async () => {},
        async () => {},
      );
      host.appendChild(el);
    }, nextWeekSecs);

    const host = page.locator('#deal-close-test-host');
    const receiveBtn = host.locator('.deal-close-receive-button');
    await expect(receiveBtn).toBeVisible();
    await receiveBtn.click();

    const closeList = host.locator('.close-list');
    await expect(closeList).toBeVisible();

    const goodsCheckbox = closeList.locator('.close-list-item-c_goods .close-list-item-checkbox');
    const lessonCheckbox = closeList.locator('.close-list-item-c_lesson .close-list-item-checkbox');

    await expect(goodsCheckbox).toBeChecked();
    await expect(lessonCheckbox).not.toBeChecked();
    await expect(closeList.locator('.close-list-item-c_lesson .close-list-not-started')).toBeVisible();

    await lessonCheckbox.check();
    await expect(lessonCheckbox).toBeChecked();

    let closePayload: any = null;
    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'agreement.close') {
        closePayload = body.params;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: { records: ['rec_c1', 'rec_c2'], skipped: [] } }),
        });
        return;
      }
      await route.fallback();
    });

    await closeList.locator('.deal-close-sign-button').click();
    await expect.poll(() => closePayload).toEqual(
      expect.objectContaining({
        agreement: 'rec_agr_close',
        commitments: ['c_goods', 'c_lesson'],
      }),
    );
  });

  test('3. multi-version quote displays change summary and suggest changes sets message prefix', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'deal-v3-test-host';
      document.body.appendChild(host);

      const chatPane = document.createElement('div');
      chatPane.className = 'chat-pane';
      const composeInput = document.createElement('input');
      composeInput.className = 'compose-input';
      chatPane.appendChild(composeInput);
      host.appendChild(chatPane);

      const cardWrap = document.createElement('div');
      chatPane.appendChild(cardWrap);

      const summary = {
        mine: {
          added: [{ resource_kind: 'goods', description: 'Wood glue' }],
          removed: [],
          changed: [],
          totalBefore: 10000n,
          totalAfter: 15000n,
        },
        theirs: {
          added: [],
          removed: [],
          changed: [],
          totalBefore: 10000n,
          totalAfter: 10000n,
        },
      };

      const cardEl = win.RoymRegistry.renderCard({
        type: 'quote',
        version: 1,
        data: {
          record_id: 'rec_quote_v3',
          version_number: 3,
          summary,
          terms: {
            currency: 'INR',
            commitments: [
              {
                commitment_id: 'c1',
                giver: 'quote-author',
                resource_kind: 'service',
                description: { lang: 'en', text: 'Plumbing' },
                declared_value_minor: 10000,
              },
            ],
          },
        },
      });
      cardWrap.appendChild(cardEl);

      win.RoymMessages.renderCardActions(
        cardWrap,
        {
          message_id: 'msg_quote_v3',
          conversation: 'conv_deal',
          direction: 'incoming',
          card_type: 'quote',
          version: 1,
          record_id: 'rec_quote_v3',
          version_count: 3,
          stored_at_secs: 1000,
          data: {
            counterparty_did: 'did:key:author',
            terms: { currency: 'INR' },
          },
        },
        'conv_deal',
        'did:key:viewer',
        async () => {},
      );
    });

    const host = page.locator('#deal-v3-test-host');
    await expect(host.locator('.quote-version')).toContainText('Version 3');
    await expect(host.locator('.quote-changes-summary')).toBeVisible();
    await expect(host.locator('.quote-changes-heading')).toBeVisible();

    const suggestBtn = host.locator('.quote-suggest-changes-button');
    await expect(suggestBtn).toBeVisible();
    await suggestBtn.click();

    const composeInput = host.locator('.compose-input');
    await expect(composeInput).toHaveValue('About offer version 3: ');
  });

  test('4. quote form with prefilled listing sends with only scope entered', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    let quoteSetParams: any = null;
    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'deal.defaults-get') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              cancellation_terms: { lang: 'en', text: 'Default cancellation' },
              refund_terms: { lang: 'en', text: 'Default refund' },
              dispute_path: { lang: 'en', text: 'Default dispute' },
              quote_valid_secs: 172800,
            },
          }),
        });
        return;
      }
      if (body?.method === 'quote.set') {
        quoteSetParams = body.params;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: { quote_record_id: 'rec_quote_d4' } }),
        });
        return;
      }
      await route.fallback();
    });

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'deal-listing-test-host';
      document.body.appendChild(host);

      const reqCard = {
        card_type: 'enquiry',
        record_id: 'rec_enquiry_1',
        data: {
          listing_id: 'list_woodwork',
        },
      };

      const preFill = {
        terms: {
          currency: 'INR',
          amount_minor: 50000,
          payee: 'did:key:author',
          commitments: [
            {
              commitment_id: 'c1',
              giver: 'quote-author',
              kind: 'service',
              resource_kind: 'service',
              declared_value_minor: 50000,
              location: { where: 'at-giver' },
            },
            {
              commitment_id: 'c2',
              giver: 'counterparty',
              kind: 'currency',
              resource_kind: 'currency',
              declared_value_minor: 50000,
              payee: 'did:key:author',
            },
          ],
        },
      };

      win.RoymMessages.openQuoteForm(host, reqCard, 'conv_deal', async () => {}, preFill);
    });

    const host = page.locator('#deal-listing-test-host');
    const form = host.locator('.quote-form');
    await expect(form).toBeVisible();

    const moreTerms = form.locator('.quote-more-terms');
    await expect(moreTerms).not.toHaveAttribute('open', '');

    await form.locator('.quote-scope-input').fill('Full woodwork workshop');
    await form.locator('.submit-quote-button').click();

    await expect.poll(() => quoteSetParams).toBeTruthy();
    expect(quoteSetParams.terms.scope).toEqual(
      expect.objectContaining({ text: 'Full woodwork workshop' }),
    );
    expect(quoteSetParams.terms.currency).toBe('INR');
  });

  test('5. statement screen loads exported records and supports csv and json downloads', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'statement.export') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              statement_version: 1,
              owner_did: 'did:key:alice',
              from_secs: body.params.from_secs,
              to_secs: body.params.to_secs,
              generated_at_secs: Math.floor(Date.now() / 1000),
              rows: [
                {
                  agreement: 'rec_agr_1',
                  commitment_id: 'c1',
                  agreed_at_secs: 1700000000,
                  settled_at_secs: 1700050000,
                  direction: 'given',
                  counterparty_did: 'did:key:bob',
                  counterparty_name: 'Bob',
                  resource_kind: 'service',
                  commitment_kind: 'fulfilment',
                  description: { lang: 'en', text: 'Woodwork lesson' },
                  currency: 'INR',
                  declared_value_minor: 30000,
                  records: ['rec_1'],
                },
              ],
              open_at_end: [],
              totals: [
                {
                  currency: 'INR',
                  direction: 'given',
                  resource_kind: 'service',
                  sum_minor: 30000,
                },
              ],
              eeps: { status: 'not-available' },
            },
          }),
        });
        return;
      }
      if (body?.method === 'contacts.list') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({ jsonrpc: '2.0', id: body.id, result: [] }),
        });
        return;
      }
      await route.fallback();
    });

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'statement-test-host';
      document.body.appendChild(host);
      win.RoymDeals.renderStatement(host);
    });

    const host = page.locator('#statement-test-host');
    await host.locator('.statement-show-button').click();

    await expect(host.locator('.statement-results')).toBeVisible();
    await expect(host.locator('.statement-row')).toBeVisible();

    const [csvDownload] = await Promise.all([
      page.waitForEvent('download'),
      host.locator('.statement-download-csv').click(),
    ]);
    expect(csvDownload.suggestedFilename()).toBe('statement.csv');

    const [jsonDownload] = await Promise.all([
      page.waitForEvent('download'),
      host.locator('.statement-download-json').click(),
    ]);
    expect(jsonDownload.suggestedFilename()).toBe('statement.json');
  });
});
