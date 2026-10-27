import * as fs from 'fs';
import * as path from 'path';
import {
  MINIAPP_BIN, MINIAPP_WASM, MINIAPP_WASM_DIR,
  aliasOf, bootNode, buildArtifacts, didOf, packMiniappAssets, spawnLogged,
} from './node-boot';
import { reserveTcpPort } from './ports';
import { waitForHttp } from './wait';

const TEST_DIR = path.join(process.cwd(), '.e2e-data');

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup ---');

  // Clean previous run data
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

  buildArtifacts({
    miniappClient: true,
    miniappWasm: true,
    roym: true,
    binaries: ['roymctl', 'substrate', 'miniapp'],
  });
  const assetsArchive = path.join(TEST_DIR, 'miniapp-demo1-wasm-assets.tar.gz');
  packMiniappAssets(assetsArchive);

  console.log('Starting Substrate...');
  const miniappPortRes = await reserveTcpPort();
  const node = await bootNode('substrate', {
    dir: TEST_DIR,
    logPath: path.join(TEST_DIR, 'substrate.log'),
    nickname: 'e2e-tester',
    registry: true,
    coordinator: {},
    gateway: { login: true },
    parent: 'self',
    interfaces: ['webrtc', 'iroh'],
    claim: true,
    injectKek: true,
  });
  (global as any).__SUBSTRATE_PROCESS__ = node.process;
  const ports = {
    registryPort: node.ports.registry!,
    irohHttpPort: node.ports.irohHttp!,
    irohQuicPort: node.ports.irohQuic!,
    webrtcSignalingPort: node.ports.webrtcSignaling!,
    webrtcBootstrapPort: node.ports.webrtcBootstrap!,
    gatewayPort: node.ports.gateway!,
    miniappPort: miniappPortRes.port,
  };
  fs.writeFileSync(path.join(TEST_DIR, 'ports.json'), JSON.stringify(ports, null, 2));
  console.log('Allocated dynamic ports:', JSON.stringify(ports));

  console.log('Starting miniapp-demo1-web...');
  await miniappPortRes.release();
  const miniappLogPath = path.join(TEST_DIR, 'miniapp.log');
  const miniappProcess = spawnLogged(
    MINIAPP_BIN,
    ['--port', ports.miniappPort.toString(), '--https-port', '0', '--data-dir', path.join(TEST_DIR, 'miniapp-data')],
    miniappLogPath);
  (global as any).__MINIAPP_PROCESS__ = miniappProcess;
  miniappProcess.on('exit', (code) => {
    if (code !== 0 && code !== null) {
      console.error(`miniapp-demo1-web exited early with code ${code}; see ${miniappLogPath}`);
    }
  });
  // Poll the port the next steps use rather than sleeping blindly: if the
  // miniapp failed to start (e.g. a port collision) this surfaces it here
  // instead of 18 tests later.
  await waitForHttp(`http://127.0.0.1:${ports.miniappPort}/`, 20000);

  const substrateDid = node.did;
  const registryUrl = node.registryUrl;

  // Generate Identity for the App
  console.log('Creating app identity...');
  node.ctl('identity create --name demo1');
  const appDid = didOf(node, 'demo1');
  console.log('App DID:', appDid);
  const appAlias = aliasOf(appDid, 'demo1', 'http');
  console.log('App Alias:', appAlias);

  // Register in Community Registry FIRST
  console.log('Registering service in Community Registry...');
  node.ctl(`--api-url ${registryUrl} registry register --identity demo1 --substrate ${substrateDid} --nickname demo1`);

  console.log('Deploying TCP Service (Passthrough)...');
  try {
    await new Promise(r => setTimeout(r, 2000));
    node.owner(`svc deploy --svc-id ${appDid} --interfaces http --tcp 127.0.0.1:${ports.miniappPort}`);
  } catch (err: any) {
    console.error("Deploy failed!");
    throw err;
  }

  // WASM App Setup
  console.log('Creating WASM app identity...');
  node.ctl('identity create --name demo1wasm');
  const wasmDid = didOf(node, 'demo1wasm');
  console.log('WASM App DID:', wasmDid);

  console.log('Registering WASM service in Community Registry...');
  node.ctl(`--api-url ${registryUrl} registry register --identity demo1wasm --substrate ${substrateDid} --nickname demo1wasm`);

  // `--interface http-native`: the HTTP bridge (assets, guest routes, SSE,
  // WebSocket) is reached through the reserved native-capability interface, the
  // same one crates/substrate/tests/*_e2e.rs put in their preamble. An
  // app-declared interface would resolve to a WasmChannel pipeline and break
  // static-asset serving, which reaches blob bytes through native dispatch.
  const wasmAlias = aliasOf(wasmDid, 'demo1wasm', 'http-native');
  console.log('WASM App Alias:', wasmAlias);

  const WASM_IFACES = [
    'syneroym:http/incoming-handler@0.1.0',
    'syneroym:http/websocket-handler@0.1.0',
    'syneroym:messaging/guest-api@0.1.0',
    'syneroym:messaging/stream-types@0.1.0',
  ].join(',');

  console.log('Deploying WASM Service...');
  node.owner(
    `svc deploy --svc-id ${wasmDid} --interfaces ${WASM_IFACES} --wasm "${MINIAPP_WASM}" ` +
    `--assets "${assetsArchive}" --asset-visibility public ` +
    `--custom-config "${path.join(MINIAPP_WASM_DIR, 'routes.json')}"`);

  // --- Roym Hub: deploy the six-service SynApp and mint a delegated key ---

  // Deploy all six services through the real manifest so depends_on,
  // bindings, and topology_visibility are exercised in the browser path.
  // --mint-masters writes one member master per service to
  // <dir>/identities/member-roym#<service>-0.key; the manifest's
  // source/assets paths are workspace-root-relative, hence cwd: WORKSPACE_DIR.
  console.log('Deploying the Roym app (six services)...');
  node.owner(
    `app deploy roym crates/roym_core/app/roym.toml ` +
    `--mint-masters --registry-url ${registryUrl} ` +
    `--journal-path "${path.join(TEST_DIR, 'roym-deployments.db')}"`);

  const roymWebDid = didOf(node, 'member-roym#web-0');
  console.log('Roym Web DID:', roymWebDid);

  // The Roym directory service's own DID. The Hub's Directory tab has no
  // way to learn its own node's directory address, so the browser suite
  // adds it as a source itself (a single-node loopback: the proxy resolves
  // a service hosted on this node through the local endpoint registry, no
  // network hop) to exercise search, publish-to-a-directory, and the
  // publication limit against a real SynOrg.
  const roymDirectoryDid = didOf(node, 'member-roym#directory-0');
  console.log('Roym Directory DID:', roymDirectoryDid);

  // `app deploy` registers the member master with no nickname (a separate,
  // operator-set annotation); add the one the gateway hostname scheme needs.
  console.log('Registering the Roym web service nickname...');
  node.ctl(
    `--api-url ${registryUrl} registry register ` +
    `--identity "member-roym#web-0" --substrate ${substrateDid} --nickname roym`);

  const roymWebAlias = aliasOf(roymWebDid, 'roym', 'http-native');
  console.log('Roym Web Alias:', roymWebAlias);

  // Mint a short-lived delegated key for owner and publish her master anchor,
  // so the Hub's delegated-key login has a session-key.json to import.
  const sessionKeyFile = path.join(TEST_DIR, 'alice-session-key.json');
  const gatewayUrl = `http://127.0.0.1:${ports.gatewayPort}`;
  console.log('Minting a delegated session key for owner...');
  node.ctl(`--as owner session delegate --registry-url ${registryUrl} --out "${sessionKeyFile}"`);

  console.log('Logging in session for owner...');
  node.ctl(`--as owner session login --gateway-url ${gatewayUrl} --registry-url ${registryUrl}`);

  console.log('Enrolling record signing certificate for owner...');
  node.ctl(
    `--as owner roym enrol-signing --master owner --gateway-url ${gatewayUrl} ` +
    `--host ${roymWebAlias} --registry-url ${registryUrl}`);

  // Set environment variables for tests
  process.env.SUBSTRATE_DID = substrateDid;
  process.env.APP_DID = appDid;
  process.env.APP_ALIAS = appAlias;
  process.env.WASM_APP_DID = wasmDid;
  process.env.WASM_APP_ALIAS = wasmAlias;
  process.env.ROYM_WEB_DID = roymWebDid;
  process.env.ROYM_DIRECTORY_DID = roymDirectoryDid;
  process.env.ROYM_WEB_ALIAS = roymWebAlias;
  process.env.ROYM_HUB_URL = `http://${roymWebAlias}:${ports.gatewayPort}`;
  process.env.ROYM_SESSION_KEY_FILE = sessionKeyFile;

  console.log('--- E2E Global Setup Complete ---\n');
}
