import { execSync, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';

const TEST_DIR = path.join(process.cwd(), '.e2e-data-multihop');

export default async function globalSetup() {
  console.log('\n--- E2E Global Setup (Multi-Hop) ---');
  
  if (fs.existsSync(TEST_DIR)) {
    fs.rmSync(TEST_DIR, { recursive: true, force: true });
  }
  fs.mkdirSync(TEST_DIR, { recursive: true });

  const WORKSPACE_DIR = path.resolve(process.cwd(), '../../../../');
  const SUBSTRATE_BIN = path.join(WORKSPACE_DIR, 'target/debug/syneroym-substrate');
  const ROYMCTL_BIN = path.join(WORKSPACE_DIR, 'target/debug/roymctl');
  const MINIAPP_BIN = path.join(WORKSPACE_DIR, 'target/debug/miniapp-demo1-web');
  
  console.log('Building Cargo binaries...');
  execSync('cargo build --bin roymctl', { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync('cargo build --bin syneroym-substrate', { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync('cargo build -p miniapp-demo1-web', { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Initialize node directories
  console.log('Initializing node directories...');
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/c`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/cp`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/sz`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}/sx`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // C config
  const configC = `
config_version = 1
app_config_dir = "${TEST_DIR}/c"
app_local_data_dir = "${TEST_DIR}/c"
app_data_dir = "${TEST_DIR}/c"
profile = "full"

[identity]
key = "identity.key"
nickname = "c-global"

[roles.community_registry]
access = "everyone"
http_bind_address = "0.0.0.0:7661"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:7664"
quic_bind_address = "0.0.0.0:7665"
community_registry_url = "http://127.0.0.1:7661"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:7663"
bootstrap_page_bind_address = "0.0.0.0:7662"

[roles.client_gateway]
http_port = 7660

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:7661"
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
key = "identity.key"
nickname = "cp-private"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:7676"
quic_bind_address = "0.0.0.0:7677"
community_registry_url = "http://127.0.0.1:7661"
share_in_registry = true

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:7673"
bootstrap_page_bind_address = "0.0.0.0:7672"

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
  fs.writeFileSync(path.join(TEST_DIR, 'cp.toml'), configCp);

  // Sz config
  const configSz = `
config_version = 1
app_config_dir = "${TEST_DIR}/sz"
app_local_data_dir = "${TEST_DIR}/sz"
app_data_dir = "${TEST_DIR}/sz"
profile = "full"

[identity]
key = "identity.key"
nickname = "sz-appnode"

[roles.app_sandbox]

[parent_coordinator.iroh]
url = "http://127.0.0.1:7664"

[parent_coordinator.webrtc]
signaling_url = "ws://127.0.0.1:7673/ws"
bootstrap_url = "ws://127.0.0.1:7672"
stun_servers = ["stun:stun.l.google.com:19302"]

[substrate]
communication_interfaces = ["webrtc", "iroh"]
registry_url = "http://127.0.0.1:7661"
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
key = "identity.key"
nickname = "sx-appnode"

[roles.app_sandbox]

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
  fs.writeFileSync(path.join(TEST_DIR, 'sx.toml'), configSx);

  console.log('Starting Coordinator C...');
  const cProcess = spawn(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'c.toml')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' }
  });
  (global as any).__C_PROCESS__ = cProcess;
  cProcess.stdout.on('data', data => process.stdout.write('[C] ' + data.toString()));
  cProcess.stderr.on('data', data => process.stdout.write('[C ERR] ' + data.toString()));

  await new Promise(r => setTimeout(r, 4000)); // Wait for C to start

  console.log('Starting Coordinator Cp...');
  const cpProcess = spawn(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'cp.toml')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' }
  });
  (global as any).__CP_PROCESS__ = cpProcess;
  cpProcess.stdout.on('data', data => process.stdout.write('[Cp] ' + data.toString()));
  cpProcess.stderr.on('data', data => process.stdout.write('[Cp ERR] ' + data.toString()));

  await new Promise(r => setTimeout(r, 4000)); // Wait for Cp to start

  console.log('Starting Sz Substrate...');
  const szProcess = spawn(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'sz.toml')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' }
  });
  (global as any).__SZ_PROCESS__ = szProcess;

  let szDid = '';
  let szOutputBuffer = '';
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Timeout waiting for Sz DID')), 15000);
    szProcess.stdout.on('data', (data) => {
      const output = data.toString();
      szOutputBuffer += output;
      process.stdout.write('[Sz] ' + output);
      const match = szOutputBuffer.match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
      if (match && !szDid) {
        szDid = match[1];
        clearTimeout(timer);
        resolve();
      }
    });
    szProcess.stderr.on('data', data => process.stdout.write('[Sz ERR] ' + data.toString()));
    szProcess.on('error', err => { clearTimeout(timer); reject(err); });
  });
  console.log('Sz DID:', szDid);

  console.log('Starting Sx Substrate...');
  const sxProcess = spawn(SUBSTRATE_BIN, ['run', '--config', path.join(TEST_DIR, 'sx.toml')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' }
  });
  (global as any).__SX_PROCESS__ = sxProcess;

  let sxDid = '';
  let sxOutputBuffer = '';
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Timeout waiting for Sx DID')), 15000);
    sxProcess.stdout.on('data', (data) => {
      const output = data.toString();
      sxOutputBuffer += output;
      process.stdout.write('[Sx] ' + output);
      const match = sxOutputBuffer.match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
      if (match && !sxDid) {
        sxDid = match[1];
        clearTimeout(timer);
        resolve();
      }
    });
    sxProcess.stderr.on('data', data => process.stdout.write('[Sx ERR] ' + data.toString()));
    sxProcess.on('error', err => { clearTimeout(timer); reject(err); });
  });
  console.log('Sx DID:', sxDid);

  // Spawn a single miniapp demo1 on port 3000 (shared target for Sz and Sx)
  console.log('Starting miniapp on port 3000...');
  const miniapp1Process = spawn(MINIAPP_BIN, ['--port', '3000', '--data-dir', path.join(TEST_DIR, 'miniapp-data1')], {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info' }
  });
  (global as any).__MINIAPP1_PROCESS__ = miniapp1Process;
  miniapp1Process.stdout.on('data', data => process.stdout.write('[Miniapp1] ' + data.toString()));
  miniapp1Process.stderr.on('data', data => process.stdout.write('[Miniapp1 ERR] ' + data.toString()));

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

  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz --api-url http://127.0.0.1:7661 registry register --identity demo1 --substrate ${szDid} --nickname demo1`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sz --api-url http://127.0.0.1:7661 --substrate ${szDid} app deploy --app-id ${did1} --interfaces http --tcp 127.0.0.1:3000`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

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

  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx --api-url http://127.0.0.1:7661 registry register --identity demo2 --substrate ${sxDid} --nickname demo2`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR}/sx --api-url http://127.0.0.1:7661 --substrate ${sxDid} app deploy --app-id ${did2} --interfaces http --tcp 127.0.0.1:3000`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Set env vars for Playwright specs
  process.env.SZ_DID = szDid;
  process.env.SX_DID = sxDid;
  process.env.DEMO1_DID = did1;
  process.env.DEMO1_ALIAS = alias1;
  process.env.DEMO2_DID = did2;
  process.env.DEMO2_ALIAS = alias2;

  console.log('--- E2E Global Setup Complete (Multi-Hop) ---\n');
}
