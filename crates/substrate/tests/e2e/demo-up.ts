import { ENV_FILE, bootDemoNodes } from './demo-nodes';

// "mise run demo:up": start S1 and S2, write the env file, and leave them running.
bootDemoNodes(true).then(
  () => {
    console.log(`\nDemo nodes are up. Env file: ${ENV_FILE}`);
    console.log('Use it in a shell with: source ' + ENV_FILE);
    console.log('Stop the nodes with: mise run demo:down');
  },
  (err) => {
    console.error(err);
    process.exit(1);
  },
);
