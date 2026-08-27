import { execSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const TEST_DIR = path.join(process.cwd(), '.e2e-data');

export default async function globalTeardown() {
  console.log('\n--- E2E Global Teardown ---');
  
  const substrateProcess = (global as any).__SUBSTRATE_PROCESS__;
  if (substrateProcess) {
    console.log('Killing Substrate Process...');
    substrateProcess.kill('SIGKILL');
  }

  const miniappProcess = (global as any).__MINIAPP_PROCESS__;
  if (miniappProcess) {
    console.log('Killing Miniapp Process...');
    miniappProcess.kill('SIGKILL');
  }

  // Keep the substrate/miniapp logs (global-setup writes them here) after the
  // data dir is wiped -- they are the only record of a server-side failure
  // during a test, and CI uploads them as an artifact.
  const LOG_DIR = path.join(process.cwd(), 'e2e-logs');
  if (fs.existsSync(TEST_DIR)) {
    fs.mkdirSync(LOG_DIR, { recursive: true });
    for (const name of ['substrate.log', 'miniapp.log']) {
      const src = path.join(TEST_DIR, name);
      if (fs.existsSync(src)) fs.copyFileSync(src, path.join(LOG_DIR, name));
    }
  }

  console.log('Cleaning up test data directory...');
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }

  console.log('--- E2E Global Teardown Complete ---\n');
}
