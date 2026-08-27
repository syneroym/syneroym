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
# The node's top-level data dir (the same path roymctl takes as --dir); the
# gateway itself appends the "identities/" subdirectory that
# "roymctl identity create" writes keys into.
person_identities_dir = "${TEST_DIR}"

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
  // mid-step, the substrate would fill its ~8 KiB stdout buffer while logging
  // (the six-service Roym `app deploy` alone emits far more than that), block
  // in a synchronous write inside its deploy handler, and never answer the
  // deploy RPC. A file write never blocks, so the deadlock cannot form.
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

  // Deploys all six Roym services through the real app manifest
  // (crates/roym_core/app/roym.toml), so depends_on, bindings, and
  // topology_visibility are actually exercised in the browser path --
  // not a lone `svc deploy` of just `web`. `--mint-masters` mints one
  // member master identity per service (ADR-0020 Sec1) under
  // <dir>/identities/member-roym#<service>-0.key; source/assets paths in
  // the manifest are workspace-root-relative (its own header comment),
  // which is why this runs with cwd: WORKSPACE_DIR like every other
  // roymctl call here.
  console.log('Deploying the Roym app (all six services)...');
  const roymDeploymentJournal = path.join(TEST_DIR, 'roym-deployments.db');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 ` +
    `--substrate ${substrateDid} --as owner app deploy roym crates/roym_core/app/roym.toml ` +
    `--mint-masters --registry-url http://127.0.0.1:7661 ` +
    `--journal-path "${roymDeploymentJournal}"`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' }
  );

  const roymWebIdentityOutput = execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} identity show --name "member-roym#web-0"`,
    { cwd: WORKSPACE_DIR }
  ).toString();
  const roymWebDidMatch = roymWebIdentityOutput.match(/(did:key:[a-z0-9]+)/);
  if (!roymWebDidMatch) throw new Error("Could not find Roym web DID in roymctl output");
  const roymWebDid = roymWebDidMatch[1];
  console.log('Roym Web DID:', roymWebDid);

  // `app deploy` mints and registers the member master itself, but with no
  // nickname (nickname/privacy are a deliberately separate, operator-set
  // registry annotation -- see deferred-backlog.md's "Endpoint-record
  // metadata" row). Add the nickname the gateway hostname scheme needs.
  console.log('Registering the Roym web service nickname...');
  execSync(
    `"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 registry register ` +
    `--identity "member-roym#web-0" --substrate ${substrateDid} --nickname roym`,
    { cwd: WORKSPACE_DIR, stdio: 'inherit' }
  );

  const roymWebAliasOutput = execSync(
    `"${ROYMCTL_BIN}" alias ${roymWebDid} --nickname roym --interface http-native`,
    { cwd: WORKSPACE_DIR }
  ).toString().trim();
  const roymWebAlias = roymWebAliasOutput.split('\n').pop()?.trim();
  if (!roymWebAlias) throw new Error("Could not calculate Roym web app alias");
  console.log('Roym Web Alias:', roymWebAlias);

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
