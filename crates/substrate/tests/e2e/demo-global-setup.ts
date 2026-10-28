import { attachToDemoNodes, bootDemoNodes, readDemoEnv } from './demo-nodes';

// With DEMO_ATTACH=1 the nodes come from "mise run demo:up" and are left
// running. Otherwise this boots its own pair and the teardown stops them.
export default async function demoGlobalSetup() {
  console.log('\n--- Demo Global Setup ---');
  if (process.env.DEMO_ATTACH === '1') {
    await attachToDemoNodes();
    console.log('Attached to the running demo nodes.');
    return;
  }
  const started = Date.now();
  await bootDemoNodes(false);
  console.log(`Demo nodes ready in ${Math.round((Date.now() - started) / 1000)} s.`);
  // The teardown stops exactly these two processes.
  const env = readDemoEnv();
  (global as any).__DEMO_PIDS__ = { S1_PID: env.S1_PID, S2_PID: env.S2_PID };
}
