import { test } from '@playwright/test';
import { readDemoEnv } from '../demo-nodes';
import { requireEnv, runScript } from './steps';

const SETUP_SCRIPT = '20-roym-setup.sh';

// The story follows "Build a Roym client" in docs/developer-guide.md. The script runs the
// guide's own tagged code blocks with curl, so the guide and the test cannot drift apart.
test('Roym client story: follow the client guide with curl', async () => {
  const env = readDemoEnv();
  for (const key of ['ALICE_KEY', 'ALICE_DID', 'BOB_ADDR', 'BOB_HUB_URL']) {
    requireEnv(env, key, SETUP_SCRIPT);
  }
  runScript('30-roym-client.sh');
});
