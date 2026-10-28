import * as fs from 'fs';
import { DEMO_DIR, ENV_FILE, readDemoEnv, stopDemoNodes } from './demo-nodes';

// "mise run demo:down": stop the nodes that "mise run demo:up" started.
async function main() {
  if (!fs.existsSync(ENV_FILE)) {
    console.log(`No env file at ${ENV_FILE}: nothing to stop.`);
    fs.rmSync(DEMO_DIR, { recursive: true, force: true });
    return;
  }
  const env = readDemoEnv();
  await stopDemoNodes({ S1_PID: env.S1_PID, S2_PID: env.S2_PID });
  console.log('Demo nodes stopped.');
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
