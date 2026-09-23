import { ChildProcess, execSync, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { reserveTcpPort, reserveUdpPort } from './ports';

const TEST_DIR = path.join(process.cwd(), '.e2e-data-multihop');
const LOG_DIR = path.join(process.cwd(), 'e2e-logs');
const WORKSPACE_DIR = path.resolve(process.cwd(), '../../../../');

function logPath(name: string): string {
  return path.join(LOG_DIR, `multihop-${name}.log`);
}

// Each node logs to its own file under `e2e-logs/`, not to the Playwright
// output. Four nodes at `info` buried the test results, and CI uploads
// `e2e-logs/` anyway. A file also never blocks the writer: a pipe is not
// drained while `execSync` freezes node's event loop, so a chatty node
// could stall on a full pipe in the middle of an RPC.
function spawnLogged(bin: string, args: string[], name: string): ChildProcess {
  fs.mkdirSync(LOG_DIR, { recursive: true });
  const fd = fs.openSync(logPath(name), 'w');
  const child = spawn(bin, args, {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' },
    stdio: ['ignore', fd, fd],
  });
  fs.closeSync(fd);
  return child;
}

// The node prints its DID once its identity is ready; poll its log for it.
async function waitForDid(child: ChildProcess, name: string): Promise<string> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`${name} exited with code ${child.exitCode}; see ${logPath(name)}`);
    }
    const match = fs.readFileSync(logPath(name), 'utf8')
      .match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
    if (match) return match[1];
    await new Promise(r => setTimeout(r, 200));
  }
  throw new Error(`Timeout waiting for ${name} DID; see ${logPath(name)}`);
}

// Offline-first install so a warm cache costs ~100 ms, not a registry
// round trip that can eat the suite's `globalTimeout` (see global-setup.ts).
const NPM_INSTALL = 'npm install --prefer-offline --no-audit --no-fund';

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup (Multi-Hop) ---');
  
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

  console.log('Allocating dynamic ports for Multi-Hop...');
  const cRegistryPortRes = await reserveTcpPort();
  const cIrohHttpPortRes = await reserveTcpPort();
  const cIrohQuicPortRes = await reserveUdpPort();
  const cWebrtcSigPortRes = await reserveTcpPort();
  const cWebrtcBootPortRes = await reserveTcpPort();
  const cGatewayPortRes = await reserveTcpPort();

  const cpIrohHttpPortRes = await reserveTcpPort();
  const cpIrohQuicPortRes = await reserveUdpPort();
  const cpWebrtcSigPortRes = await reserveTcpPort();
  const cpWebrtcBootPortRes = await reserveTcpPort();

  const miniappPortRes = await reserveTcpPort();

  const ports = {
    cRegistryPort: cRegistryPortRes.port,
    cIrohHttpPort: cIrohHttpPortRes.port,
    cIrohQuicPort: cIrohQuicPortRes.port,
    cWebrtcSigPort: cWebrtcSigPortRes.port,
    cWebrtcBootPort: cWebrtcBootPortRes.port,
    cGatewayPort: cGatewayPortRes.port,
    cpIrohHttpPort: cpIrohHttpPortRes.port,
    cpIrohQuicPort: cpIrohQuicPortRes.port,
    cpWebrtcSigPort: cpWebrtcSigPortRes.port,
    cpWebrtcBootPort: cpWebrtcBootPortRes.port,
    miniappPort: miniappPortRes.port,
  };

  const portsJsonPath = path.join(TEST_DIR, 'ports.json');
  fs.writeFileSync(portsJsonPath, JSON.stringify(ports, null, 2));
  console.log('Allocated dynamic ports for Multi-Hop:', JSON.stringify(ports));

  const isRelease = process.env.CARGO_RELEASE_FLAG === '--release';
  const targetDir = isRelease ? 'target/release' : 'target/debug';
  const buildFlag = isRelease ? '--release' : '';
  const SUBSTRATE_BIN = path.join(WORKSPACE_DIR, targetDir, 'syneroym-substrate');
  const ROYMCTL_BIN = path.join(WORKSPACE_DIR, targetDir, 'roymctl');
  const MINIAPP_BIN = path.join(WORKSPACE_DIR, targetDir, 'miniapp-demo1-web');
  
  console.log('Building miniapp SolidJS client...');
  const clientDir = path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-web/client');
  execSync(`${NPM_INSTALL} && npm run build`, { cwd: clientDir, stdio: 'inherit' });

  console.log('Building Cargo binaries...');
  execSync(`cargo build ${buildFlag} --bin roymctl`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`cargo build ${buildFlag} --bin syneroym-substrate`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`cargo build ${buildFlag} -p miniapp-demo1-web`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Initialize node directories
  console.log('Initializing node directories...');
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/c`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/cp`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/sz`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/sx`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // An unowned substrate now fails closed. Only `sz` and
  // `sx` ever receive a deploy below, so only they need claiming -- `c`
  // and `cp` are coordinator/relay only and stay unowned, which is fine
  // and cheaper (nothing deploys to them, so nothing needs `orchestrator`
  // or `security` access there).
  console.log('Creating operator identities and claiming sz/sx...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz identity create --name owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz substrate claim --controller owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx identity create --name owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx substrate claim --controller owner`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // C config
  const configC = `
config_version = 1
app_config_dir = "${TEST_DIR}/c"
app_local_data_dir = "${TEST_DIR}/c"
app_data_dir = "${TEST_DIR}/c"
profile = "full"

[identity]
key = "substrate.key"
nickname = "c-global"

[roles.community_registry]
access = "everyone"
http_bind_address = "0.0.0.0:${ports.cRegistryPort}"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:${ports.cIrohHttpPort}"
quic_bind_address = "0.0.0.0:${ports.cIrohQuicPort}"
info_http_bind_address = "0.0.0.0:0"
community_registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:${ports.cWebrtcSigPort}"
bootstrap_page_bind_address = "0.0.0.0:${ports.cWebrtcBootPort}"

[roles.client_gateway]
http_port = ${ports.cGatewayPort}

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
`;
  fs.writeFileSync(path.join(TEST_DIR, 'c.toml'), configC);

  // Cp config
  const configCp = `
config_version = 1
app_config_dir = "${TEST_DIR}/cp"
app_local_data_dir = "${TEST_DIR}/cp"
app_data_dir = "${TEST_DIR}/cp"
profile = "full"

[identity]
key = "substrate.key"
nickname = "cp-private"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:${ports.cpIrohHttpPort}"
quic_bind_address = "0.0.0.0:${ports.cpIrohQuicPort}"
info_http_bind_address = "0.0.0.0:0"
community_registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:${ports.cpWebrtcSigPort}"
bootstrap_page_bind_address = "0.0.0.0:${ports.cpWebrtcBootPort}"

[parent_coordinator.iroh]
url = "http://127.0.0.1:${ports.cIrohHttpPort}"

[parent_coordinator.webrtc]
signaling_url = "ws://127.0.0.1:${ports.cWebrtcSigPort}/ws"
bootstrap_url = "ws://127.0.0.1:${ports.cWebrtcBootPort}"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
`;
  fs.writeFileSync(path.join(TEST_DIR, 'cp.toml'), configCp);

  // Sz config
  const configSz = `
config_version = 1
app_config_dir = "${TEST_DIR}/sz"
app_local_data_dir = "${TEST_DIR}/sz"
app_data_dir = "${TEST_DIR}/sz"
profile = "full"

[identity]
key = "substrate.key"
nickname = "sz-appnode"

[roles.app_sandbox]

[parent_coordinator.iroh]
url = "http://127.0.0.1:${ports.cIrohHttpPort}"

[parent_coordinator.webrtc]
signaling_url = "ws://127.0.0.1:${ports.cpWebrtcSigPort}/ws"
bootstrap_url = "ws://127.0.0.1:${ports.cpWebrtcBootPort}"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
`;
  fs.writeFileSync(path.join(TEST_DIR, 'sz.toml'), configSz);

  // Sx config
  const configSx = `
config_version = 1
app_config_dir = "${TEST_DIR}/sx"
app_local_data_dir = "${TEST_DIR}/sx"
app_data_dir = "${TEST_DIR}/sx"
profile = "full"

[identity]
key = "substrate.key"
nickname = "sx-appnode"

[roles.app_sandbox]

[parent_coordinator.iroh]
url = "http://127.0.0.1:${ports.cIrohHttpPort}"

[parent_coordinator.webrtc]
signaling_url = "ws://127.0.0.1:${ports.cWebrtcSigPort}/ws"
bootstrap_url = "ws://127.0.0.1:${ports.cWebrtcBootPort}"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:${ports.cRegistryPort}"
`;
  fs.writeFileSync(path.join(TEST_DIR, 'sx.toml'), configSx);

  console.log('Starting Coordinator C...');
  await Promise.all([
    cRegistryPortRes.release(),
    cIrohHttpPortRes.release(),
    cIrohQuicPortRes.release(),
    cWebrtcSigPortRes.release(),
    cWebrtcBootPortRes.release(),
    cGatewayPortRes.release(),
  ]);
  const cProcess = spawnLogged(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'c.toml')], 'c');
  (global as any).__C_PROCESS__ = cProcess;

  await new Promise(r => setTimeout(r, 4000)); // Wait for C to start

  console.log('Starting Coordinator Cp...');
  await Promise.all([
    cpIrohHttpPortRes.release(),
    cpIrohQuicPortRes.release(),
    cpWebrtcSigPortRes.release(),
    cpWebrtcBootPortRes.release(),
  ]);
  const cpProcess = spawnLogged(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'cp.toml')], 'cp');
  (global as any).__CP_PROCESS__ = cpProcess;

  await new Promise(r => setTimeout(r, 4000)); // Wait for Cp to start

  console.log('Starting Sz Substrate...');
  const szProcess = spawnLogged(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'sz.toml')], 'sz');
  (global as any).__SZ_PROCESS__ = szProcess;
  const szDid = await waitForDid(szProcess, 'sz');
  console.log('Sz DID:', szDid);

  console.log('Starting Sx Substrate...');
  const sxProcess = spawnLogged(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'sx.toml')], 'sx');
  (global as any).__SX_PROCESS__ = sxProcess;
  const sxDid = await waitForDid(sxProcess, 'sx');
  console.log('Sx DID:', sxDid);

  // Spawn a single miniapp demo1 on dynamic port (shared target for Sz and Sx)
  console.log(`Starting miniapp on port ${ports.miniappPort}...`);
  await miniappPortRes.release();
  const miniapp1Process = spawnLogged(MINIAPP_BIN, ['--port', ports.miniappPort.toString(), '--https-port', '0', '--data-dir', path.join(TEST_DIR, 'miniapp-data1')], 'miniapp1');
  (global as any).__MINIAPP1_PROCESS__ = miniapp1Process;

  await new Promise(r => setTimeout(r, 4000));

  // Initialize and Register demo1 for Sz
  console.log('Creating demo1 identity (Sz)...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz identity create --name demo1`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  const idOutput1 = execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz identity show --name demo1`, { cwd: WORKSPACE_DIR }).toString();
  const did1 = idOutput1.match(/(did:key:[a-z0-9]+)/)?.[1];
  if (!did1) throw new Error("Could not find demo1 DID");
  const aliasOutput1 = execSync(`"${ROYMCTL_BIN}" alias ${did1} --nickname demo1 --interface http`, { cwd: WORKSPACE_DIR }).toString().trim();
  const alias1 = aliasOutput1.split('\n').pop()?.trim();
  if (!alias1) throw new Error("Could not calculate demo1 alias");
  console.log('Demo1 App DID:', did1, 'Alias:', alias1);

  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz --api-url http://127.0.0.1:${ports.cRegistryPort} registry register --identity demo1 --substrate ${szDid} --nickname demo1`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz --api-url http://127.0.0.1:${ports.cRegistryPort} --substrate ${szDid} --as owner svc deploy --svc-id ${did1} --interfaces http --tcp 127.0.0.1:${ports.miniappPort}`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Initialize and Register demo2 for Sx
  console.log('Creating demo2 identity (Sx)...');
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx identity create --name demo2`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  const idOutput2 = execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx identity show --name demo2`, { cwd: WORKSPACE_DIR }).toString();
  const did2 = idOutput2.match(/(did:key:[a-z0-9]+)/)?.[1];
  if (!did2) throw new Error("Could not find demo2 DID");
  const aliasOutput2 = execSync(`"${ROYMCTL_BIN}" alias ${did2} --nickname demo2 --interface http`, { cwd: WORKSPACE_DIR }).toString().trim();
  const alias2 = aliasOutput2.split('\n').pop()?.trim();
  if (!alias2) throw new Error("Could not calculate demo2 alias");
  console.log('Demo2 App DID:', did2, 'Alias:', alias2);

  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx --api-url http://127.0.0.1:${ports.cRegistryPort} registry register --identity demo2 --substrate ${sxDid} --nickname demo2`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx --api-url http://127.0.0.1:${ports.cRegistryPort} --substrate ${sxDid} --as owner svc deploy --svc-id ${did2} --interfaces http --tcp 127.0.0.1:${ports.miniappPort}`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Set env vars for Playwright specs
  process.env.SZ_DID = szDid;
  process.env.SX_DID = sxDid;
  process.env.DEMO1_DID = did1;
  process.env.DEMO1_ALIAS = alias1;
  process.env.DEMO2_DID = did2;
  process.env.DEMO2_ALIAS = alias2;

  console.log('--- E2E Global Setup Complete (Multi-Hop) ---\n');
}
