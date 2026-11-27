import fs from 'node:fs';
import { test, expect } from '../fixtures';
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
          verified: true,
          data: {
            counterparty_did: 'did:key:viewer',
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

  test('2. agreement panel displays per-commitment actions, waiting state, and closing buttons', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    let closePayload: any = null;
    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'agreement.close') {
        closePayload = body.params;
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              records: ['rec_closed_c1'],
              skipped: [],
              remaining: 0,
            },
          }),
        });
        return;
      }
      await route.fallback();
    });

    await page.evaluate(() => {
      const win = window as any;
      const host = document.createElement('div');
      host.id = 'deal-panel-test-host';
      document.body.appendChild(host);

      const view = {
        quote_record_id: 'rec_quote_agr_1',
        quote_author_did: 'did:key:author',
        counterparty_did: 'did:key:viewer',
        state: 'agreed',
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
          ],
        },
        commitments: [
          {
            commitment_id: 'c1',
            state: 'promised',
          },
          {
            commitment_id: 'c2',
            state: 'promised',
          },
        ],
      };

      const ownHalves = new Map([['c2', 'rec_half_c2_signed']]);
      const panelEl = win.RoymDeals.renderAgreementPanelView(
        view,
        'did:key:viewer',
        async () => {},
        async () => {},
        ownHalves,
      );
      host.appendChild(panelEl);
    });

    const host = page.locator('#deal-panel-test-host');
    await expect(host.locator('.agreement-commitment-c1 .deal-action-received-button')).toBeVisible();
    await expect(host.locator('.agreement-commitment-c2 .deal-waiting-other-node')).toBeVisible();

    const giveAllBtn = host.locator('.deal-close-give-button');
    await expect(giveAllBtn).toBeVisible();
    await giveAllBtn.click();

    const closeList = host.locator('.close-list');
    await expect(closeList).toBeVisible();

    const signBtn = closeList.locator('.deal-close-sign-button');
    await signBtn.click();

    await expect.poll(() => closePayload).toBeTruthy();
    expect(closePayload.as).toBe('giver');
    expect(closePayload.commitments).toContain('c2');
  });

  test('3. multi-version quote displays change summary and suggest changes sets message prefix', async ({ page }) => {
    await page.goto(HUB_URL);
    await page.waitForLoadState('networkidle');

    await page.route('**/rpc', async (route) => {
      const body = route.request().postDataJSON();
      if (body?.method === 'quote.history') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              versions: [
                {
                  quote_id: 'qid_deal_v3',
                  sequence: 1,
                  terms: {
                    currency: 'INR',
                    commitments: [
                      {
                        commitment_id: 'c1',
                        giver: 'quote-author',
                        resource_kind: 'service',
                        kind: 'service',
                        description: { lang: 'en', text: 'Plumbing basic' },
                        declared_value_minor: 10000,
                      },
                    ],
                  },
                },
                {
                  quote_id: 'qid_deal_v3',
                  sequence: 2,
                  terms: {
                    currency: 'INR',
                    commitments: [
                      {
                        commitment_id: 'c1',
                        giver: 'quote-author',
                        resource_kind: 'service',
                        kind: 'service',
                        description: { lang: 'en', text: 'Plumbing standard' },
                        declared_value_minor: 12000,
                      },
                    ],
                  },
                },
                {
                  quote_id: 'qid_deal_v3',
                  sequence: 3,
                  terms: {
                    currency: 'INR',
                    commitments: [
                      {
                        commitment_id: 'c1',
                        giver: 'quote-author',
                        resource_kind: 'service',
                        kind: 'service',
                        description: { lang: 'en', text: 'Plumbing complete' },
                        declared_value_minor: 15000,
                      },
                    ],
                  },
                },
              ],
            },
          }),
        });
        return;
      }
      await route.fallback();
    });

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

      const msgRow = {
        id: 'msg_quote_v3',
        conversation: 'conv_deal',
        direction: 'incoming',
        content_type: 'application/vnd.roym.card+json',
        state: 'delivered',
      };

      const cardRow = {
        message_id: 'msg_quote_v3',
        conversation: 'conv_deal',
        direction: 'incoming',
        card_type: 'quote',
        version: 1,
        record_id: 'rec_quote_v3',
        version_count: 3,
        stored_at_secs: 1000,
        known: true,
        verified: true,
        data: {
          quote_id: 'qid_deal_v3',
          counterparty_did: 'did:key:viewer',
          terms: {
            currency: 'INR',
            commitments: [
              {
                commitment_id: 'c1',
                giver: 'quote-author',
                resource_kind: 'service',
                description: { lang: 'en', text: 'Plumbing complete' },
                declared_value_minor: 15000,
              },
            ],
          },
        },
      };

      const cardMap = new Map([['msg_quote_v3', cardRow]]);
      const msgEl = win.RoymMessages.messageElement(
        msgRow,
        cardMap,
        [cardRow],
        new Map(),
        new Map([['qid_deal_v3', 'rec_quote_v3']]),
        'conv_deal',
        'did:key:viewer',
        async () => {},
      );
      cardWrap.appendChild(msgEl);
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
      if (body?.method === 'listing.get') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              envelope: 'env_woodwork',
            },
          }),
        });
        return;
      }
      if (body?.method === 'listing.verify') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
              verified: true,
              payload: {
                listing_id: 'list_woodwork',
                payment: { amount_minor: 50000, currency: 'INR', payee: 'did:key:author' },
                deal_terms: {
                  cancellation_terms: { lang: 'en', text: 'Listing cancellation' },
                  refund_terms: { lang: 'en', text: 'Listing refund' },
                  dispute_path: { lang: 'en', text: 'Listing dispute' },
                },
              },
            },
          }),
        });
        return;
      }
      if (body?.method === 'deal.defaults-get') {
        await route.fulfill({
          status: 200,
          contentType: 'application/json',
          body: JSON.stringify({
            jsonrpc: '2.0',
            id: body.id,
            result: {
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

      win.RoymMessages.openQuoteForm(host, reqCard, 'conv_deal', async () => {}, undefined, 'did:key:author');
    });

    const host = page.locator('#deal-listing-test-host');
    const form = host.locator('.quote-form');
    await expect(form).toBeVisible();

    await expect(form.locator('.quote-amount-input')).toHaveValue('500.00');
    await expect(form.locator('.quote-payee-input')).toHaveValue('did:key:author');
    await expect(form.locator('.quote-canc-input')).toHaveValue('Listing cancellation');

    const moreTerms = form.locator('.quote-more-terms');
    await expect(moreTerms).not.toHaveAttribute('open', '');

    await form.locator('.quote-scope-input').fill('Full woodwork workshop');
    await form.locator('.submit-quote-button').click();

    await expect.poll(() => quoteSetParams).toBeTruthy();
    expect(quoteSetParams.terms.scope).toEqual(
      expect.objectContaining({ text: 'Full woodwork workshop' }),
    );
    expect(quoteSetParams.terms.currency).toBe('INR');
    expect(quoteSetParams.terms.cancellation_terms).toEqual(
      expect.objectContaining({ text: 'Listing cancellation' }),
    );
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
    const csvPath = await csvDownload.path();
    expect(csvPath).toBeTruthy();
    const csvContent = fs.readFileSync(csvPath!, 'utf-8');
    const csvLines = csvContent.split('\r\n');
    expect(csvLines[0]).toBe('agreement,commitment_id,agreed_at,settled_at,direction,counterparty_did,counterparty_name,resource_kind,commitment_kind,description,currency,declared_value,settlement_note');
    expect(csvLines[1]).toContain('rec_agr_1,c1,');
    expect(csvLines[1]).toContain('given');
    expect(csvLines[1]).toContain('did:key:bob');
    expect(csvLines[1]).toContain('300.00');

    const [jsonDownload] = await Promise.all([
      page.waitForEvent('download'),
      host.locator('.statement-download-json').click(),
    ]);
    expect(jsonDownload.suggestedFilename()).toBe('statement.json');
    const jsonPath = await jsonDownload.path();
    expect(jsonPath).toBeTruthy();
    const jsonContent = JSON.parse(fs.readFileSync(jsonPath!, 'utf-8'));
    expect(jsonContent.owner_did).toBe('did:key:alice');
    expect(jsonContent.rows).toHaveLength(1);
  });
});
