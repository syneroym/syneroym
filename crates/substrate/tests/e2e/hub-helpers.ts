import { expect, type Page } from './fixtures';

const SESSION_KEY_FILE = process.env.ROYM_SESSION_KEY_FILE;

// Drive the real delegated-key login: hand the Hub the session-key.json that
// `global-setup`'s `roymctl session delegate` produced, exactly as a person
// would after running the command themselves.
export async function loginWithDelegatedKey(page: Page) {
  await expect(page.locator('.login-picker h2')).toHaveText('Sign in');
  expect(SESSION_KEY_FILE, 'ROYM_SESSION_KEY_FILE must be set by global-setup').toBeTruthy();
  await page.locator('input[type="file"]').setInputFiles(SESSION_KEY_FILE!);
  await expect(page.locator('.session-bar')).toContainText('did:key:', { timeout: 15_000 });
  await expect(page.locator('.session-bar')).toContainText('delegated');
}

// One JSON-RPC method over the logged-in session's own bearer token, the
// same shape scenario 1's own whoami check uses -- for a setup step the
// Hub has no screen for.
export async function rpcCall(page: Page, method: string, params: unknown) {
  return page.evaluate(
    async ({ method, params }) => {
      const win = window as unknown as {
        RoymSession?: { authHeaders: () => Record<string, string> };
      };
      const headers = win.RoymSession?.authHeaders() ?? {};
      const res = await fetch('/rpc', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json', ...headers },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
      });
      return res.json();
    },
    { method, params },
  );
}
