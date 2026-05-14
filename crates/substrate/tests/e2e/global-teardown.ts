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

  console.log('Cleaning up test data directory...');
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }

  console.log('--- E2E Global Teardown Complete ---\n');
}
