import * as fs from 'fs';
import * as path from 'path';

const TEST_DIR = path.join(process.cwd(), '.e2e-data-multihop');

export default async function globalTeardown() {
  console.log('\n--- E2E Global Teardown (Multi-Hop) ---');
  
  const cProcess = (global as any).__C_PROCESS__;
  if (cProcess) {
    console.log('Killing C Process...');
    cProcess.kill('SIGKILL');
  }

  const cpProcess = (global as any).__CP_PROCESS__;
  if (cpProcess) {
    console.log('Killing Cp Process...');
    cpProcess.kill('SIGKILL');
  }

  const szProcess = (global as any).__SZ_PROCESS__;
  if (szProcess) {
    console.log('Killing Sz Process...');
    szProcess.kill('SIGKILL');
  }

  const sxProcess = (global as any).__SX_PROCESS__;
  if (sxProcess) {
    console.log('Killing Sx Process...');
    sxProcess.kill('SIGKILL');
  }

  const miniapp1Process = (global as any).__MINIAPP1_PROCESS__;
  if (miniapp1Process) {
    console.log('Killing Miniapp 1 Process...');
    miniapp1Process.kill('SIGKILL');
  }

  const miniapp2Process = (global as any).__MINIAPP2_PROCESS__;
  if (miniapp2Process) {
    console.log('Killing Miniapp 2 Process...');
    miniapp2Process.kill('SIGKILL');
  }

  console.log('Cleaning up test data directory...');
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }

  console.log('--- E2E Global Teardown Complete (Multi-Hop) ---\n');
}
