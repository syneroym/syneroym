import { DemoPids, stopDemoNodes } from './demo-nodes';

export default async function demoGlobalTeardown() {
  console.log('\n--- Demo Global Teardown ---');
  const pids: DemoPids | undefined = (global as any).__DEMO_PIDS__;
  if (!pids) {
    console.log('Attached to running nodes; leaving them up.');
    return;
  }
  await stopDemoNodes(pids);
  console.log('--- Demo Global Teardown Complete ---\n');
}
