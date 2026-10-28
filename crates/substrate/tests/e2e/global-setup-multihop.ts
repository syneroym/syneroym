import * as fs from 'fs';
import * as path from 'path';
import {
  MINIAPP_BIN, NodeHandle, aliasOf, bootNode, buildArtifacts, didOf, parentOf, spawnLogged,
} from './node-boot';
import { reserveTcpPort } from './ports';
import { waitForHttp } from './wait';

const TEST_DIR = path.join(process.cwd(), '.e2e-data-multihop');
// Each node logs to its own file under `e2e-logs/`, not to the Playwright
// output. Four nodes at `info` buried the test results, and CI uploads
// `e2e-logs/` anyway.
const LOG_DIR = path.join(process.cwd(), 'e2e-logs');

function logPath(name: string): string {
  return path.join(LOG_DIR, `multihop-${name}.log`);
}

// Deploy a TCP passthrough service for the shared mini-app on `node`.
function deployDemoApp(node: NodeHandle, name: string, miniappPort: number) {
  console.log(`Creating ${name} identity (${node.name})...`);
  node.ctl(`identity create --name ${name}`);
  const did = didOf(node, name);
  const alias = aliasOf(did, name, 'http');
  console.log(`${name} App DID:`, did, 'Alias:', alias);

  node.ctl(`--api-url ${node.registryUrl} registry register --identity ${name} --substrate ${node.did} --nickname ${name}`);
  node.owner(`svc deploy --svc-id ${did} --interfaces http --tcp 127.0.0.1:${miniappPort}`);
  return { did, alias };
}

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup (Multi-Hop) ---');

  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

  buildArtifacts({ miniappClient: true, binaries: ['roymctl', 'substrate', 'miniapp'] });

  const miniappPortRes = await reserveTcpPort();

  console.log('Starting Coordinator C...');
  const c = await bootNode('c', {
    dir: path.join(TEST_DIR, 'c'),
    logPath: logPath('c'),
    nickname: 'c-global',
    registry: true,
    coordinator: { shareInRegistry: true },
    gateway: {},
    interfaces: ['webrtc'],
    awaitRegistration: false,
  });
  (global as any).__C_PROCESS__ = c.process;
  await waitForHttp(`http://127.0.0.1:${c.ports.webrtcBootstrap}/`, 20_000);

  console.log('Starting Coordinator Cp...');
  const cp = await bootNode('cp', {
    dir: path.join(TEST_DIR, 'cp'),
    logPath: logPath('cp'),
    nickname: 'cp-private',
    coordinator: { shareInRegistry: true },
    registryUrl: c.registryUrl,
    parent: parentOf(c.ports),
    interfaces: ['webrtc', 'iroh'],
    awaitRegistration: false,
  });
  (global as any).__CP_PROCESS__ = cp.process;
  await waitForHttp(`http://127.0.0.1:${cp.ports.webrtcBootstrap}/`, 20_000);

  // Only `sz` and `sx` ever receive a deploy, so only they are claimed. `c`
  // and `cp` are coordinator/relay only and stay unowned: nothing deploys to
  // them, so nothing needs `orchestrator` or `security` access there.
  // Sz reaches the network through Cp for WebRTC; Sx goes straight to C.
  const viaC = parentOf(c.ports);
  const viaCp = parentOf(cp.ports);
  console.log('Starting Sz Substrate...');
  const sz = await bootNode('sz', {
    dir: path.join(TEST_DIR, 'sz'),
    logPath: logPath('sz'),
    nickname: 'sz-appnode',
    appSandbox: {},
    registryUrl: c.registryUrl,
    parent: { ...viaC, webrtcSignalingUrl: viaCp.webrtcSignalingUrl, webrtcBootstrapUrl: viaCp.webrtcBootstrapUrl },
    interfaces: ['webrtc', 'iroh'],
    claim: true,
  });
  (global as any).__SZ_PROCESS__ = sz.process;

  console.log('Starting Sx Substrate...');
  const sx = await bootNode('sx', {
    dir: path.join(TEST_DIR, 'sx'),
    logPath: logPath('sx'),
    nickname: 'sx-appnode',
    appSandbox: {},
    registryUrl: c.registryUrl,
    parent: viaC,
    interfaces: ['webrtc', 'iroh'],
    claim: true,
  });
  (global as any).__SX_PROCESS__ = sx.process;

  // One mini-app on a dynamic port is the shared target for Sz and Sx.
  const miniappPort = miniappPortRes.port;
  console.log(`Starting miniapp on port ${miniappPort}...`);
  await miniappPortRes.release();
  (global as any).__MINIAPP1_PROCESS__ = spawnLogged(
    MINIAPP_BIN,
    ['--port', miniappPort.toString(), '--https-port', '0', '--data-dir', path.join(TEST_DIR, 'miniapp-data1')],
    logPath('miniapp1'));
  await waitForHttp(`http://127.0.0.1:${miniappPort}/`, 20000);

  const demo1 = deployDemoApp(sz, 'demo1', miniappPort);
  const demo2 = deployDemoApp(sx, 'demo2', miniappPort);

  const ports = {
    cRegistryPort: c.ports.registry,
    cIrohHttpPort: c.ports.irohHttp,
    cIrohQuicPort: c.ports.irohQuic,
    cWebrtcSigPort: c.ports.webrtcSignaling,
    cWebrtcBootPort: c.ports.webrtcBootstrap,
    cGatewayPort: c.ports.gateway,
    cpIrohHttpPort: cp.ports.irohHttp,
    cpIrohQuicPort: cp.ports.irohQuic,
    cpWebrtcSigPort: cp.ports.webrtcSignaling,
    cpWebrtcBootPort: cp.ports.webrtcBootstrap,
    miniappPort,
  };
  fs.writeFileSync(path.join(TEST_DIR, 'ports.json'), JSON.stringify(ports, null, 2));
  console.log('Allocated dynamic ports for Multi-Hop:', JSON.stringify(ports));

  // Set env vars for Playwright specs
  process.env.SZ_DID = sz.did;
  process.env.SX_DID = sx.did;
  process.env.DEMO1_DID = demo1.did;
  process.env.DEMO1_ALIAS = demo1.alias;
  process.env.DEMO2_DID = demo2.did;
  process.env.DEMO2_ALIAS = demo2.alias;

  console.log('--- E2E Global Setup Complete (Multi-Hop) ---\n');
}
