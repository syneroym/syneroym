import { execSync, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const TEST_DIR = path.join(process.cwd(), '.e2e-data');

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup ---');
  
  // Clean previous run data
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

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
  execSync('npm install && npm run build', { cwd: clientDir, stdio: 'inherit' });

  console.log('Building miniapp-demo1-wasm client + component...');
  // Order matters: src/lib.rs include_str!s static/dist/index.html, which the
  // client build regenerates with a fresh hashed bundle name.
  execSync('npm ci || npm install', { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
  execSync('npm run build', { cwd: path.join(WASM_FIXTURE_DIR, 'client'), stdio: 'inherit' });
  // Always --release: that is the path crates/core/src/test_constants.rs names,
  // and the fixture is excluded from the workspace build graph.
  execSync('cargo component build --release --target wasm32-wasip2',
           { cwd: WASM_FIXTURE_DIR, stdio: 'inherit' });

  if (!fs.existsSync(WASM_ARTIFACT)) {
    throw new Error(`WASM artifact was not found at expected path: ${WASM_ARTIFACT}`);
  }

  fs.mkdirSync(TEST_DIR, { recursive: true });
  const assetsArchive = path.join(TEST_DIR, 'miniapp-demo1-wasm-assets.tar.gz');
  // COPYFILE_DISABLE stops macOS tar from adding ._ AppleDouble entries, which
  // would land in the manifest as junk paths.
  execSync(`COPYFILE_DISABLE=1 tar -czf "${assetsArchive}" -C "${path.join(WASM_FIXTURE_DIR, 'static')}" .`,
           { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Building Roym Hub UI bundle...');
  const roymUiDir = path.join(WORKSPACE_DIR, 'crates/roym_web/ui');
  execSync('npm run build && npm run pack', { cwd: roymUiDir, stdio: 'inherit' });

  console.log('Building Roym WASM components...');
  const roymServices = ['profile', 'conversation', 'catalog', 'transaction', 'directory', 'web'];
  for (const svc of roymServices) {
    execSync(`cargo component build --release --target wasm32-wasip2 -p syneroym-roym-${svc}`, {
      cwd: WORKSPACE_DIR,
      stdio: 'inherit',
    });
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
person_identities_dir = "${path.join(TEST_DIR, 'identities')}"

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
  const substrateProcess = spawn(SUBSTRATE_BIN, ['run', '--config', configPath], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' }
  });
  (global as any).__SUBSTRATE_PROCESS__ = substrateProcess;

  let substrateDid = '';
  let substrateOutputBuffer = '';
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Timeout waiting for substrate DID')), 20000);

    substrateProcess.stdout.on('data', (data) => {
      const output = data.toString();
      substrateOutputBuffer += output;
      process.stdout.write('[Substrate] ' + output);
      const match = substrateOutputBuffer.match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
      if (match && !substrateDid) {
        substrateDid = match[1];
        clearTimeout(timer);
        resolve();
      }
    });
    substrateProcess.stderr.on('data', (data) => {
      process.stdout.write('[Substrate ERR] ' + data.toString());
    });
    substrateProcess.on('error', (err) => {
      clearTimeout(timer);
      reject(err);
    });
  });

  console.log('Substrate DID extracted:', substrateDid);

  console.log('Starting miniapp-demo1-web...');
  const miniappProcess = spawn(MINIAPP_BIN, ['--port', '3000', '--data-dir', path.join(TEST_DIR, 'miniapp-data')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info' }
  });
  (global as any).__MINIAPP_PROCESS__ = miniappProcess;
  
  miniappProcess.stdout.on('data', data => process.stdout.write('[Miniapp] ' + data.toString()));
  miniappProcess.stderr.on('data', data => process.stdout.write('[Miniapp ERR] ' + data.toString()));

  console.log('Waiting for components to be ready...');
  await new Promise(r => setTimeout(r, 4000));

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

  // Roym Hub & Services Setup
  console.log('Creating alice person identity...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name alice`, {
    cwd: WORKSPACE_DIR,
    stdio: 'inherit',
  });

  console.log('Creating Roym web service identity...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity create --name roym_web`, {
    cwd: WORKSPACE_DIR,
    stdio: 'inherit',
  });
  const roymWebIdentityOutput = execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name roym_web`,
    { cwd: WORKSPACE_DIR }
  ).toString();
  const roymWebDidMatch = roymWebIdentityOutput.match(/(did:key:[a-z0-9]+)/);
  if (!roymWebDidMatch) throw new Error("Could not find Roym web DID in roymctl output");
  const roymWebDid = roymWebDidMatch[1];
  console.log('Roym Web DID:', roymWebDid);

  console.log('Registering Roym web service in Community Registry...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 registry register --identity roym_web --substrate ${substrateDid} --nickname roym`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' }
  );

  const roymWebAliasOutput = execSync(
    `"${ROYMCTL_BIN}" alias ${roymWebDid} --nickname roym --interface http-native`,
    { cwd: WORKSPACE_DIR }
  ).toString().trim();
  const roymWebAlias = roymWebAliasOutput.split('\n').pop()?.trim();
  if (!roymWebAlias) throw new Error("Could not calculate Roym web app alias");
  console.log('Roym Web Alias:', roymWebAlias);

  console.log('Deploying Roym Web Service...');
  const roymWebWasm = path.join(WORKSPACE_DIR, 'target/wasm32-wasip2/release/syneroym_roym_web.wasm');
  const roymWebAssets = path.join(WORKSPACE_DIR, 'crates/roym_web/ui/bundle.tar.gz');
  const roymRoutes = path.join(TEST_DIR, 'roym-web-routes.json');
  fs.writeFileSync(roymRoutes, JSON.stringify({
    http_routes: [
      { method: 'GET', path: '/health', target: 'guest', operation: 'handle-request', public: true },
      { method: 'POST', path: '/rpc', target: 'guest', operation: 'handle-request', public: false },
    ]
  }));

  // Matches crates/roym_web/wit/world.wit's actual exports -- web declares
  // no messaging interfaces.
  const ROYM_WEB_IFACES = [
    'syneroym:http/incoming-handler@0.1.0',
    'syneroym:http/websocket-handler@0.1.0',
  ].join(',');

  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
    `--substrate ${substrateDid} --as owner svc deploy --svc-id ${roymWebDid} ` +
    `--interfaces ${ROYM_WEB_IFACES} --wasm "${roymWebWasm}" ` +
    `--assets "${roymWebAssets}" --asset-visibility public ` +
    `--custom-config "${roymRoutes}"`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' }
  );

  // Set environment variables for tests
  process.env.SUBSTRATE_DID = substrateDid;
  process.env.APP_DID = appDid;
  process.env.APP_ALIAS = appAlias;
  process.env.WASM_APP_DID = wasmDid;
  process.env.WASM_APP_ALIAS = wasmAlias;
  process.env.ROYM_WEB_DID = roymWebDid;
  process.env.ROYM_WEB_ALIAS = roymWebAlias;
  process.env.ROYM_HUB_URL = `http://${roymWebAlias}:7660`;

  console.log('--- E2E Global Setup Complete ---\n');
}
