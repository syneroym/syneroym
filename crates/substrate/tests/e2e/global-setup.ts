import { execSync, spawn } from 'child_process';
import * as fs from 'fs';
import * as net from 'net';
import * as path from 'path';

const TEST_DIR = path.join(process.cwd(), '.e2e-data');

// The whole suite runs under one `globalTimeout` (300 s). A plain
// `npm install` still makes registry round trips even when everything is
// already on disk -- observed at ~3 min each on a network-restricted host,
// which alone exhausted the budget before the substrate started. These
// flags keep it offline-first (~100 ms with a warm cache) and drop the
// audit/funds calls that have no place in a test build.
const NPM_INSTALL = 'npm install --prefer-offline --no-audit --no-fund';
const NPM_CI = 'npm ci --prefer-offline --no-audit --no-fund';

// Ports the harness binds. If a previous run was killed before global-teardown
// ran (so its substrate / miniapp were never reaped), one of these is still
// held -- the miniapp then panics on its second `bind`, the tests run against
// the zombie whose SQLite file this setup just deleted, and only the
// DB-touching cases (POST, WS broadcast) fail, which looks like a WebRTC bug.
// Fail loudly here instead.
const REQUIRED_PORTS = [3000, 3001, 7660, 7661, 7662, 7663, 7664, 7665];

function portInUse(port: number): Promise<boolean> {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once('error', (e: NodeJS.ErrnoException) => resolve(e.code === 'EADDRINUSE'));
    srv.once('listening', () => srv.close(() => resolve(false)));
    srv.listen(port, '0.0.0.0');
  });
}

async function preflightPorts(): Promise<void> {
  const busy: number[] = [];
  for (const p of REQUIRED_PORTS) {
    if (await portInUse(p)) busy.push(p);
  }
  if (busy.length > 0) {
    throw new Error(
      `E2E ports already in use: ${busy.join(', ')}. A previous run likely ` +
      `left an orphaned substrate/miniapp. Kill it (e.g. ` +
      `\`lsof -ti tcp:${busy[0]} | xargs kill -9\`) and re-run.`);
  }
}

async function waitForHttp(url: string, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try {
      const res = await fetch(url);
      if (res.ok || res.status === 404) return;
    } catch {
      // not up yet
    }
    if (Date.now() > deadline) throw new Error(`Timed out waiting for ${url}`);
    await new Promise(r => setTimeout(r, 200));
  }
}

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup ---');
  
  // Clean previous run data
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

  await preflightPorts();

  const WORKSPACE_DIR = path.resolve(process.cwd(), '../../../../');
  const isRelease = process.env.CARGO_RELEASE_FLAG === '--release';
  const targetDir = isRelease ? 'target/release' : 'target/debug';
  const buildFlag = isRelease ? '--release' : '';
  const SUBSTRATE_BIN = path.join(WORKSPACE_DIR, targetDir, 'syneroym-substrate');
  const ROYMCTL_BIN = path.join(WORKSPACE_DIR, targetDir, 'roymctl');
  const MINIAPP_BIN = path.join(WORKSPACE_DIR, targetDir, 'miniapp-demo1-web');
  const WASM_FIXTURE_DIR = path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-wasm');
  const WASM_ARTIFACT = path.join(
    WASM_FIXTURE_DIR, 'target/wasm32-wasip2/release/syneroym_test_miniapp_demo1_wasm.wasm');
  
  console.log('Building miniapp SolidJS client...');
  const clientDir = path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-web/client');
  execSync(`${NPM_INSTALL} && npm run build`, { cwd: clientDir, stdio: 'inherit' });

  console.log('Building miniapp-demo1-wasm client + component...');
  // Order matters: src/lib.rs include_str!s static/dist/index.html, which the
  // client build regenerates with a fresh hashed bundle name.
  execSync(`${NPM_CI} || ${NPM_INSTALL}`, { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
  execSync('npm run build', { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
  // Always --release: that is the path crates/core/src/test_constants.rs names,
  // and the fixture is excluded from the workspace build graph.
  execSync('cargo component build --release --target wasm32-wasip2',
           { cwd: WASM_FIXTURE_DIR, stdio: 'inherit' });

  if (!fs.existsSync(WASM_ARTIFACT)) {
    throw new Error(`WASM artifact was not found at expected path: ${WASM_ARTIFACT}`);
  }

  const assetsArchive = path.join(TEST_DIR, 'miniapp-demo1-wasm-assets.tar.gz');
  // COPYFILE_DISABLE stops macOS tar from adding ._ AppleDouble entries, which
  // would land in the manifest as junk paths.
  execSync(`COPYFILE_DISABLE=1 tar -czf "${assetsArchive}" -C "${path.join(WASM_FIXTURE_DIR, 'static')}" .`,
           { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Building the Roym Hub UI bundle and service components...');
  execSync('npm run build && npm run pack',
           { cwd: path.join(WORKSPACE_DIR, 'crates/roym_web/ui'), stdio: 'inherit' });
  for (const svc of ['profile', 'conversation', 'catalog', 'transaction', 'directory', 'web']) {
    execSync(`cargo component build --release --target wasm32-wasip2 -p syneroym-roym-${svc}`,
             { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  }

  console.log('Building Cargo binaries...');
  execSync(`cargo build ${buildFlag} --bin roymctl`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`cargo build ${buildFlag} --bin syneroym-substrate`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`cargo build ${buildFlag} -p miniapp-demo1-web`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Initializing local node identity...');
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // An unowned substrate now fails closed, so the operator
  // flow claims the node before it ever starts -- the real end-to-end
  // path, not `[iam].admin_ucan_root` shortcutting past it. No `agreement =`
  // line is needed below: the substrate discovers `<TEST_DIR>/agreement.json`
  // (roymctl substrate claim's default output) with no config change.
  console.log('Creating operator identity and claiming the substrate...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} substrate claim --controller owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Generate syneroym.toml overrides
  const configContent = `
config_version = 1
app_config_dir = "${TEST_DIR}"
app_local_data_dir = "${TEST_DIR}"
app_data_dir = "${TEST_DIR}"
profile = "full"

[identity]
key = "substrate.key"
nickname = "e2e-tester"

[roles.community_registry]
access = "everyone"
http_bind_address = "0.0.0.0:7661"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:7664"
quic_bind_address = "0.0.0.0:7665"

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:7663"
bootstrap_page_bind_address = "0.0.0.0:7662"

[roles.client_gateway]
http_port = 7660
identity_mode = "login"

[roles.auth]

[parent_coordinator.iroh]
url = "http://127.0.0.1:7664"

[parent_coordinator.webrtc]
signaling_url = "ws://127.0.0.1:7663/ws"
bootstrap_url = "ws://127.0.0.1:7662"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:7661"

`;
  const configPath = path.join(TEST_DIR, 'syneroym.toml');
  fs.writeFileSync(configPath, configContent);

  console.log('Starting Substrate...');
  // Send the substrate's stdout/stderr straight to a file, not to a pipe this
  // process reads. Every later step here shells out with `execSync`, which
  // freezes node's event loop -- so a captured pipe would stop being drained
  // mid-step, the substrate would fill its ~8 KiB stdout socket buffer while
  // logging, block in a synchronous write inside its deploy RPC handler, and
  // never answer the deploy. A file write never blocks, so the deadlock cannot
  // form.
  const substrateLogPath = path.join(TEST_DIR, 'substrate.log');
  const substrateLogFd = fs.openSync(substrateLogPath, 'a');
  const substrateProcess = spawn(SUBSTRATE_BIN, ['run', '--config', configPath], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' },
    stdio: ['ignore', substrateLogFd, substrateLogFd]
  });
  fs.closeSync(substrateLogFd);
  (global as any).__SUBSTRATE_PROCESS__ = substrateProcess;

  let substrateDid = '';
  await new Promise<void>((resolve, reject) => {
    const deadline = Date.now() + 20000;
    substrateProcess.on('error', reject);
    const poll = setInterval(() => {
      const log = fs.existsSync(substrateLogPath) ? fs.readFileSync(substrateLogPath, 'utf8') : '';
      const match = log.match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
      if (match) {
        substrateDid = match[1];
        clearInterval(poll);
        resolve();
      } else if (Date.now() > deadline) {
        clearInterval(poll);
        reject(new Error(`Timeout waiting for substrate DID (see ${substrateLogPath})`));
      }
    }, 200);
  });

  console.log('Substrate DID extracted:', substrateDid, '- substrate logs at', substrateLogPath);

  console.log('Starting miniapp-demo1-web...');
  // Same reason as the substrate above: a captured pipe left undrained during
  // an `execSync` step would deadlock this long-lived process on a full stdout
  // buffer. Log to a file instead.
  const miniappLogPath = path.join(TEST_DIR, 'miniapp.log');
  const miniappLogFd = fs.openSync(miniappLogPath, 'a');
  const miniappProcess = spawn(MINIAPP_BIN, ['--port', '3000', '--data-dir', path.join(TEST_DIR, 'miniapp-data')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info' },
    stdio: ['ignore', miniappLogFd, miniappLogFd]
  });
  fs.closeSync(miniappLogFd);
  (global as any).__MINIAPP_PROCESS__ = miniappProcess;
  miniappProcess.on('exit', (code) => {
    if (code !== 0 && code !== null) {
      console.error(`miniapp-demo1-web exited early with code ${code}; see ${miniappLogPath}`);
    }
  });

  console.log('Waiting for components to be ready...');
  // Poll the ports the next steps use rather than sleeping blindly: if the
  // miniapp failed to start (e.g. a port collision) or the substrate never
  // finished wiring its registry, this surfaces it here instead of 18 tests
  // later.
  await waitForHttp('http://127.0.0.1:3000/', 20000);        // miniapp
  await waitForHttp('http://127.0.0.1:7661/', 20000);        // community registry
  await new Promise(r => setTimeout(r, 1000));

  // Fixed 32-byte Key Encryption Key (KEK). Required for WASM services because
  // static asset unpacking, the data-layer store, and the blob store wrap their
  // Data Encryption Keys (DEKs) with the node's KEK before persisting blobs.
  const TEST_KEK = '21'.repeat(32);
  console.log('Injecting substrate KEK...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
           `--substrate ${substrateDid} --as owner kek inject ${TEST_KEK}`,
           { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Generate Identity for the App
  console.log('Creating app identity...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name demo1`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  
  const appIdentityOutput = execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name demo1`, { cwd: WORKSPACE_DIR }).toString();
  const appDidMatch = appIdentityOutput.match(/(did:key:[a-z0-9]+)/);
  if (!appDidMatch) throw new Error("Could not find app DID in roymctl output");
  const appDid = appDidMatch[1];
  console.log('App DID:', appDid);

  // Calculate Alias
  console.log('Calculating app alias...');
  const aliasOutput = execSync(`"${ROYMCTL_BIN}" alias ${appDid} --nickname demo1 --interface http`, { cwd: WORKSPACE_DIR }).toString().trim();
  const appAlias = aliasOutput.split('\n').pop()?.trim();
  if (!appAlias) throw new Error("Could not calculate app alias");
  console.log('App Alias:', appAlias);

  // Register in Community Registry FIRST
  console.log('Registering service in Community Registry...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 registry register --identity demo1 --substrate ${substrateDid} --nickname demo1`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Deploy Passthrough Service.
  console.log('Deploying TCP Service (Passthrough)...');
  try {
    await new Promise(r => setTimeout(r, 2000));
    execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 --substrate ${substrateDid} --as owner svc deploy --svc-id ${appDid} --interfaces http --tcp 127.0.0.1:3000`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  } catch (err: any) {
    console.error("Deploy failed!");
    throw err;
  }

  // WASM App Setup
  console.log('Creating WASM app identity...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name demo1wasm`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  const wasmIdentityOutput = execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name demo1wasm`, { cwd: WORKSPACE_DIR }).toString();
  const wasmDidMatch = wasmIdentityOutput.match(/(did:key:[a-z0-9]+)/);
  if (!wasmDidMatch) throw new Error("Could not find WASM app DID in roymctl output");
  const wasmDid = wasmDidMatch[1];
  console.log('WASM App DID:', wasmDid);

  console.log('Registering WASM service in Community Registry...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 registry register --identity demo1wasm --substrate ${substrateDid} --nickname demo1wasm`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Calculating WASM app alias...');
  // `--interface http-native`: the HTTP bridge (assets, guest routes, SSE,
  // WebSocket) is reached through the reserved native-capability interface, the
  // same one crates/substrate/tests/*_e2e.rs put in their preamble. An
  // app-declared interface would resolve to a WasmChannel pipeline and break
  // static-asset serving, which reaches blob bytes through native dispatch.
  const wasmAliasOutput = execSync(
    `"${ROYMCTL_BIN}" alias ${wasmDid} --nickname demo1wasm --interface http-native`,
    { cwd: WORKSPACE_DIR }
  ).toString().trim();
  const wasmAlias = wasmAliasOutput.split('\n').pop()?.trim();
  if (!wasmAlias) throw new Error("Could not calculate WASM app alias");
  console.log('WASM App Alias:', wasmAlias);

  const WASM_IFACES = [
    'syneroym:http/incoming-handler@0.1.0',
    'syneroym:http/websocket-handler@0.1.0',
    'syneroym:messaging/guest-api@0.1.0',
    'syneroym:messaging/stream-types@0.1.0',
  ].join(',');

  console.log('Deploying WASM Service...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
    `--substrate ${substrateDid} --as owner svc deploy --svc-id ${wasmDid} ` +
    `--interfaces ${WASM_IFACES} --wasm "${WASM_ARTIFACT}" ` +
    `--assets "${assetsArchive}" --asset-visibility public ` +
    `--custom-config "${path.join(WASM_FIXTURE_DIR, 'routes.json')}"`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // --- Roym Hub: deploy the six-service SynApp and mint a delegated key ---

  // Deploy all six services through the real manifest so depends_on,
  // bindings, and topology_visibility are exercised in the browser path.
  // --mint-masters writes one member master per service to
  // <dir>/identities/member-roym#<service>-0.key; the manifest's
  // source/assets paths are workspace-root-relative, hence cwd: WORKSPACE_DIR.
  console.log('Deploying the Roym app (six services)...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
    `--substrate ${substrateDid} --as owner app deploy roym crates/roym_core/app/roym.toml ` +
    `--mint-masters --registry-url http://127.0.0.1:7661 ` +
    `--journal-path "${path.join(TEST_DIR, 'roym-deployments.db')}"`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  const roymWebIdentityOutput = execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name "member-roym#web-0"`,
    { cwd: WORKSPACE_DIR }).toString();
  const roymWebDidMatch = roymWebIdentityOutput.match(/(did:key:[a-z0-9]+)/);
  if (!roymWebDidMatch) throw new Error('Could not find Roym web DID in roymctl output');
  const roymWebDid = roymWebDidMatch[1];
  console.log('Roym Web DID:', roymWebDid);

  // `app deploy` registers the member master with no nickname (a separate,
  // operator-set annotation); add the one the gateway hostname scheme needs.
  console.log('Registering the Roym web service nickname...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 registry register ` +
    `--identity "member-roym#web-0" --substrate ${substrateDid} --nickname roym`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  const roymWebAlias = execSync(
    `"${ROYMCTL_BIN}" alias ${roymWebDid} --nickname roym --interface http-native`,
    { cwd: WORKSPACE_DIR }).toString().trim().split('\n').pop()?.trim();
  if (!roymWebAlias) throw new Error('Could not calculate the Roym web alias');
  console.log('Roym Web Alias:', roymWebAlias);

  // Mint a short-lived delegated key for owner and publish her master anchor,
  // so the Hub's delegated-key login has a session-key.json to import.
  const sessionKeyFile = path.join(TEST_DIR, 'alice-session-key.json');
  console.log('Minting a delegated session key for owner...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --as owner session delegate ` +
    `--registry-url http://127.0.0.1:7661 --out "${sessionKeyFile}"`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Logging in session for owner...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --as owner session login ` +
    `--gateway-url http://127.0.0.1:7660 --registry-url http://127.0.0.1:7661`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Enrolling record signing certificate for owner...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --as owner roym enrol-signing ` +
    `--master owner --gateway-url http://127.0.0.1:7660 --host ${roymWebAlias} --registry-url http://127.0.0.1:7661`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Set environment variables for tests
  process.env.SUBSTRATE_DID = substrateDid;
  process.env.APP_DID = appDid;
  process.env.APP_ALIAS = appAlias;
  process.env.WASM_APP_DID = wasmDid;
  process.env.WASM_APP_ALIAS = wasmAlias;
  process.env.ROYM_WEB_DID = roymWebDid;
  process.env.ROYM_WEB_ALIAS = roymWebAlias;
  process.env.ROYM_HUB_URL = `http://${roymWebAlias}:7660`;
  process.env.ROYM_SESSION_KEY_FILE = sessionKeyFile;

  console.log('--- E2E Global Setup Complete ---\n');
}
