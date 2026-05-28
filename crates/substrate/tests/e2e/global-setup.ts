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
  const SUBSTRATE_BIN = path.join(WORKSPACE_DIR, 'target/debug/syneroym-substrate');
  const ROYMCTL_BIN = path.join(WORKSPACE_DIR, 'target/debug/roymctl');
  const MINIAPP_BIN = path.join(WORKSPACE_DIR, 'target/debug/miniapp-demo1-web');
  
  console.log('Building Cargo binaries...');
  execSync('cargo build --bin roymctl', { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync('cargo build --bin syneroym-substrate', { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  execSync('cargo build -p miniapp-demo1-web', { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  console.log('Initializing local node identity...');
  execSync(`"${ROYMCTL_BIN}" node init --dir ${TEST_DIR}`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });

  // Generate syneroym.toml overrides
  const configContent = `
config_version = 1
app_config_dir = "${TEST_DIR}"
app_local_data_dir = "${TEST_DIR}"
app_data_dir = "${TEST_DIR}"
profile = "full"

[identity]
key = "identity.key"
nickname = "e2e-tester"

[roles.community_registry]
access = "everyone"
http_bind_address = "127.0.0.1:7661"

[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "127.0.0.1:7664"
quic_bind_address = "127.0.0.1:7665"

[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "127.0.0.1:7663"
bootstrap_page_bind_address = "127.0.0.1:7662"

[roles.client_gateway]
http_port = 7660

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

  // Deploy Passthrough Service
  console.log('Deploying TCP Service (Passthrough)...');
  try {
    await new Promise(r => setTimeout(r, 2000));
    execSync(`"${ROYMCTL_BIN}" --dir ${TEST_DIR} --api-url http://127.0.0.1:7661 --substrate ${substrateDid} app deploy --app-id ${appDid} --interfaces http --tcp 127.0.0.1:3000`, { cwd: WORKSPACE_DIR, stdio: 'inherit' });
  } catch (err: any) {
    console.error("Deploy failed!");
    throw err;
  }

  // Set environment variables for tests
  process.env.SUBSTRATE_DID = substrateDid;
  process.env.APP_DID = appDid;
  process.env.APP_ALIAS = appAlias;
  
  console.log('--- E2E Global Setup Complete ---\n');
}
