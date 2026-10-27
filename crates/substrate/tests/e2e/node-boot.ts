import { ChildProcess, execSync, spawn } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import { ReservedPort, reserveTcpPort, reserveUdpPort } from './ports';
import { waitForHttp, waitForRegistryRecord } from './wait';

// Boot code shared by every Playwright setup that starts real nodes: the main
// suite, the multi-hop suite and the demo stories. It builds the artifacts the
// nodes need and starts one node at a time with the roles asked for.

export const WORKSPACE_DIR = path.resolve(__dirname, '../../../..');

const isRelease = process.env.CARGO_RELEASE_FLAG === '--release';
const TARGET_DIR = path.join(WORKSPACE_DIR, isRelease ? 'target/release' : 'target/debug');
const CARGO_PROFILE = isRelease ? '--release' : '--profile test';

export const SUBSTRATE_BIN = path.join(TARGET_DIR, 'syneroym-substrate');
export const ROYMCTL_BIN = path.join(TARGET_DIR, 'roymctl');
export const MINIAPP_BIN = path.join(TARGET_DIR, 'miniapp-demo1-web');

export const MINIAPP_WASM_DIR = path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-wasm');
export const MINIAPP_WASM = path.join(
  MINIAPP_WASM_DIR, 'target/wasm32-wasip2/release/syneroym_test_miniapp_demo1_wasm.wasm');
export const GREETER_WASM = path.join(
  WORKSPACE_DIR, 'test-components/greeter/target/wasm32-wasip2/release/syneroym_test_greeter.wasm');

// Fixed 32-byte Key Encryption Key (KEK). WASM services need it: static asset
// unpacking, the data-layer store and the blob store wrap their Data
// Encryption Keys with the node's KEK before they persist anything.
export const TEST_KEK = '21'.repeat(32);

const ROYM_SERVICES = ['profile', 'conversation', 'catalog', 'transaction', 'directory', 'web'];

// The setups run under a fixed `globalTimeout`. A plain `npm install` still
// makes registry round trips even when everything is on disk -- observed at
// ~3 min each on a network-restricted host, which alone exhausted the budget
// before a node started. These flags keep it offline-first (~100 ms with a
// warm cache) and drop the audit/funds calls that have no place in a test
// build.
const NPM_INSTALL = 'npm install --prefer-offline --no-audit --no-fund';
const NPM_CI = 'npm ci --prefer-offline --no-audit --no-fund';

/** Run a shell command from the workspace root, streaming its output. */
export function run(cmd: string, cwd = WORKSPACE_DIR): void {
  execSync(cmd, { cwd, stdio: 'inherit' });
}

/** Run a shell command from the workspace root and return its stdout. */
export function capture(cmd: string, cwd = WORKSPACE_DIR): string {
  return execSync(cmd, { cwd }).toString();
}

export interface BuildOptions {
  /** SolidJS client of the TCP passthrough mini-app. */
  miniappClient?: boolean;
  /** Client bundle and WASM component of the mini-app that runs in the sandbox. */
  miniappWasm?: boolean;
  /** Roym Hub bundle and the six Roym service components. */
  roym?: boolean;
  /** The greeter component used by the substrate demo story. */
  greeter?: boolean;
  /** Cargo binaries to build. */
  binaries: Array<'roymctl' | 'substrate' | 'miniapp'>;
}

const BINARY_BUILD_ARGS = {
  roymctl: '--bin roymctl',
  substrate: '--bin syneroym-substrate',
  miniapp: '-p miniapp-demo1-web',
} as const;

export function buildArtifacts(options: BuildOptions): void {
  if (options.miniappClient) {
    console.log('Building miniapp SolidJS client...');
    run(`${NPM_INSTALL} && npm run build`,
        path.join(WORKSPACE_DIR, 'test-components/miniapp-demo1-web/client'));
  }

  if (options.miniappWasm) buildMiniappWasm();
  if (options.roym) buildRoym();

  if (options.greeter) {
    console.log('Building the greeter component...');
    run('cargo component build --release --target wasm32-wasip2',
        path.join(WORKSPACE_DIR, 'test-components/greeter'));
    requireFile(GREETER_WASM);
  }

  console.log('Building Cargo binaries...');
  for (const bin of options.binaries) {
    run(`cargo build ${CARGO_PROFILE} ${BINARY_BUILD_ARGS[bin]}`);
  }
}

function buildMiniappWasm(): void {
  console.log('Building miniapp-demo1-wasm client + component...');
  const clientDir = path.join(MINIAPP_WASM_DIR, 'client');
  // Order matters: src/lib.rs include_str!s static/dist/index.html, which the
  // client build regenerates with a fresh hashed bundle name.
  run(`${NPM_CI} || ${NPM_INSTALL}`, clientDir);
  run('npm run build', clientDir);
  // Always --release: that is the path crates/core/src/test_constants.rs names,
  // and the fixture is excluded from the workspace build graph.
  run('cargo component build --release --target wasm32-wasip2', MINIAPP_WASM_DIR);
  requireFile(MINIAPP_WASM);
}

function buildRoym(): void {
  console.log('Building the Roym Hub UI bundle and service components...');
  // Self-contained: `npm run build` here runs eslint/tsc/vite from
  // `node_modules/.bin`, so the deps have to be installed first.
  const roymUiDir = path.join(WORKSPACE_DIR, 'crates/roym_web/ui');
  run(`${NPM_CI} || ${NPM_INSTALL}`, roymUiDir);
  run('npm run build && npm run pack', roymUiDir);
  for (const svc of ROYM_SERVICES) {
    run(`cargo component build --release --target wasm32-wasip2 -p syneroym-roym-${svc}`);
  }
}

function requireFile(file: string): void {
  if (!fs.existsSync(file)) {
    throw new Error(`Build artifact was not found at expected path: ${file}`);
  }
}

/** Pack the mini-app's static files into the archive `svc deploy --assets` takes. */
export function packMiniappAssets(archive: string): void {
  // COPYFILE_DISABLE stops macOS tar from adding ._ AppleDouble entries, which
  // would land in the manifest as junk paths.
  run(`COPYFILE_DISABLE=1 tar -czf "${archive}" -C "${path.join(MINIAPP_WASM_DIR, 'static')}" .`);
}

/**
 * Start a long-running process with its output going straight to a file.
 *
 * A captured pipe is not drained while `execSync` freezes node's event loop.
 * The child would then fill its ~8 KiB stdout socket buffer, block in a
 * synchronous write inside an RPC handler, and never answer. A file write
 * never blocks, so that deadlock cannot form.
 */
export function spawnLogged(
  bin: string, args: string[], logPath: string, detached = false,
): ChildProcess {
  fs.mkdirSync(path.dirname(logPath), { recursive: true });
  const fd = fs.openSync(logPath, 'w');
  const child = spawn(bin, args, {
    cwd: WORKSPACE_DIR,
    env: { ...process.env, RUST_LOG: 'info', NO_COLOR: '1' },
    stdio: ['ignore', fd, fd],
    detached,
  });
  fs.closeSync(fd);
  // A detached child outlives this process, so it must not keep the event loop alive.
  if (detached) child.unref();
  return child;
}

export interface ParentCoordinator {
  irohUrl: string;
  webrtcSignalingUrl: string;
  webrtcBootstrapUrl: string;
}

export interface NodeOptions {
  /** Data directory. Created by `roymctl node init`. */
  dir: string;
  /** Where the node writes its log. */
  logPath: string;
  /** Let the node keep running after this process exits. */
  detached?: boolean;
  nickname: string;
  /** The node runs the community registry. */
  registry?: boolean;
  /** The node runs the Iroh and WebRTC coordinators. `shareInRegistry` lists the Iroh relay in the registry. */
  coordinator?: { shareInRegistry?: boolean };
  /** The node runs the client gateway. `login` adds the session-login mode and the auth role. */
  gateway?: { login?: boolean };
  /** The node runs the app sandbox. `conversationTickSecs` sets the Roym outbox tick. */
  appSandbox?: { conversationTickSecs?: number };
  /** Registry the node publishes to. Defaults to its own, which needs `registry: true`. */
  registryUrl?: string;
  /** The coordinator this node connects out to. `'self'` points at the node's own coordinator. */
  parent?: ParentCoordinator | 'self';
  interfaces: string[];
  /** Create an owner identity and claim the node. An unclaimed node refuses deploys. */
  claim?: boolean;
  /** Inject the test KEK after start. Needs `claim`. */
  injectKek?: boolean;
  /** Wait until the node's DID shows up in the registry. Defaults to true. */
  awaitRegistration?: boolean;
  /** Extra TOML tables, appended to the generated config. */
  extraToml?: string;
}

export interface NodePorts {
  registry?: number;
  irohHttp?: number;
  irohQuic?: number;
  webrtcSignaling?: number;
  webrtcBootstrap?: number;
  gateway?: number;
}

export interface NodeHandle {
  name: string;
  dir: string;
  did: string;
  ports: NodePorts;
  registryUrl: string;
  process: ChildProcess;
  logPath: string;
  /** `roymctl --dir <node dir> <args>`, output streamed. */
  ctl(args: string): void;
  /** Same as `ctl`, returning stdout. */
  ctlOut(args: string): string;
  /** `roymctl` against this node as its owner: `--dir --api-url --substrate --as owner <args>`. */
  owner(args: string): void;
}

interface PortPlan {
  ports: NodePorts;
  reserved: ReservedPort[];
}

async function planPorts(options: NodeOptions): Promise<PortPlan> {
  const ports: NodePorts = {};
  const reserved: ReservedPort[] = [];
  const take = async (key: keyof NodePorts, udp = false) => {
    const r = udp ? await reserveUdpPort() : await reserveTcpPort();
    ports[key] = r.port;
    reserved.push(r);
  };
  if (options.registry) await take('registry');
  if (options.coordinator) {
    await take('irohHttp');
    await take('irohQuic', true);
    await take('webrtcSignaling');
    await take('webrtcBootstrap');
  }
  if (options.gateway) await take('gateway');
  return { ports, reserved };
}

/** The parent a node would give others that connect to its coordinator. */
export function parentOf(ports: NodePorts): ParentCoordinator {
  return {
    irohUrl: `http://127.0.0.1:${ports.irohHttp}`,
    webrtcSignalingUrl: `ws://127.0.0.1:${ports.webrtcSignaling}/ws`,
    webrtcBootstrapUrl: `ws://127.0.0.1:${ports.webrtcBootstrap}`,
  };
}

function renderConfig(options: NodeOptions, ports: NodePorts, registryUrl: string): string {
  const parts: string[] = [`
config_version = 1
app_config_dir = "${options.dir}"
app_local_data_dir = "${options.dir}"
app_data_dir = "${options.dir}"
profile = "full"

[identity]
key = "substrate.key"
nickname = "${options.nickname}"
`];

  if (options.registry) {
    parts.push(`
[roles.community_registry]
access = "everyone"
http_bind_address = "0.0.0.0:${ports.registry}"
`);
  }
  if (options.coordinator) {
    parts.push(`
[roles.coordinator.iroh]
enable_signalling = true
enable_relay = true
http_bind_address = "0.0.0.0:${ports.irohHttp}"
quic_bind_address = "0.0.0.0:${ports.irohQuic}"
info_http_bind_address = "0.0.0.0:0"
${options.coordinator.shareInRegistry ? `community_registry_url = "${registryUrl}"\nshare_in_registry = true\n` : ''}
[roles.coordinator.webrtc]
enable_signalling = true
enable_relay = true
signalling_bind_address = "0.0.0.0:${ports.webrtcSignaling}"
bootstrap_page_bind_address = "0.0.0.0:${ports.webrtcBootstrap}"
`);
  }
  if (options.gateway) {
    parts.push(`
[roles.client_gateway]
http_port = ${ports.gateway}
${options.gateway.login ? 'identity_mode = "login"\n\n[roles.auth]\n' : ''}`);
  }
  if (options.appSandbox) {
    const tick = options.appSandbox.conversationTickSecs;
    parts.push(`
[roles.app_sandbox]
${tick === undefined ? '' : `conversation_tick_secs = ${tick}\n`}`);
  }
  const parent = options.parent === 'self' ? parentOf(ports) : options.parent;
  if (parent) {
    parts.push(`
[parent_coordinator.iroh]
url = "${parent.irohUrl}"

[parent_coordinator.webrtc]
signaling_url = "${parent.webrtcSignalingUrl}"
bootstrap_url = "${parent.webrtcBootstrapUrl}"
stun_servers = ["stun:stun.l.google.com:19302"]
`);
  }
  const interfaces = options.interfaces.map(i => `"${i}"`).join(', ');
  parts.push(`
[substrate]
communication_interfaces = [${interfaces}]
registry_url = "${registryUrl}"
`);
  if (options.extraToml) parts.push(options.extraToml);
  return parts.join('');
}

// The node prints its DID once its identity is ready; poll its log for it.
async function waitForDid(child: ChildProcess, name: string, logPath: string): Promise<string> {
  const deadline = Date.now() + 20_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`${name} exited with code ${child.exitCode}; see ${logPath}`);
    }
    const log = fs.existsSync(logPath) ? fs.readFileSync(logPath, 'utf8') : '';
    const match = log.match(/substrate identity initialized(?:.*?)did:\s*(did:key:[a-z0-9]+)/i);
    if (match) return match[1];
    await new Promise(r => setTimeout(r, 200));
  }
  throw new Error(`Timeout waiting for ${name} DID; see ${logPath}`);
}

function roymctlCommand(dir: string, args: string): string {
  return `"${ROYMCTL_BIN}" --dir "${dir}" ${args}`;
}

/**
 * Initialise, configure and start one node, and wait until it is usable.
 *
 * Ports come from `ports.ts`. They stay reserved until just before the node
 * is spawned, so nothing else can take them while the config is written.
 */
export async function bootNode(name: string, options: NodeOptions): Promise<NodeHandle> {
  const roymctl = (args: string) => roymctlCommand(options.dir, args);

  fs.mkdirSync(options.dir, { recursive: true });
  run(`"${ROYMCTL_BIN}" node init --dir "${options.dir}"`);

  // An unowned substrate fails closed, so the operator flow claims the node
  // before it ever starts: the real end-to-end path, not an admin key written
  // into the config.
  if (options.claim) {
    run(roymctl('identity create --name owner'));
    run(roymctl('substrate claim --controller owner'));
  }

  const { ports, reserved } = await planPorts(options);
  const registryUrl = options.registryUrl ?? `http://127.0.0.1:${ports.registry}`;
  const configPath = path.join(options.dir, 'syneroym.toml');
  fs.writeFileSync(configPath, renderConfig(options, ports, registryUrl));

  await Promise.all(reserved.map(r => r.release()));
  const child = spawnLogged(
    SUBSTRATE_BIN, ['run', '--config', configPath], options.logPath, options.detached);
  try {
    return await finishBoot(name, options, child, ports, registryUrl);
  } catch (err) {
    child.kill('SIGKILL');
    throw err;
  }
}

async function finishBoot(
  name: string, options: NodeOptions, child: ChildProcess, ports: NodePorts, registryUrl: string,
): Promise<NodeHandle> {
  const roymctl = (args: string) => roymctlCommand(options.dir, args);
  const did = await waitForDid(child, name, options.logPath);
  console.log(`${name} DID: ${did} (log: ${options.logPath})`);

  if (ports.registry) await waitForHttp(registryUrl + '/', 20_000);
  if (options.awaitRegistration !== false) {
    await waitForRegistryRecord(`${registryUrl}/lookup/${did}`, 30_000);
  }

  const handle: NodeHandle = {
    name,
    dir: options.dir,
    did,
    ports,
    registryUrl,
    process: child,
    logPath: options.logPath,
    ctl: args => run(roymctl(args)),
    ctlOut: args => capture(roymctl(args)),
    owner: args => run(roymctl(`--api-url ${registryUrl} --substrate ${did} --as owner ${args}`)),
  };

  if (options.injectKek) {
    console.log(`Injecting ${name} KEK...`);
    handle.owner(`kek inject ${TEST_KEK}`);
  }
  return handle;
}

const DID_PATTERN = /(did:key:[a-z0-9]+)/;

/** The DID of an identity stored in a node's directory. */
export function didOf(node: Pick<NodeHandle, 'ctlOut'>, identity: string): string {
  const out = node.ctlOut(`identity show --name "${identity}"`);
  const did = out.match(DID_PATTERN)?.[1];
  if (!did) throw new Error(`Could not find the DID of "${identity}" in roymctl output`);
  return did;
}

/** The gateway host name `roymctl alias` prints for a service. */
export function aliasOf(did: string, nickname: string, iface: string): string {
  const alias = capture(`"${ROYMCTL_BIN}" alias ${did} --nickname ${nickname} --interface ${iface}`)
    .trim().split('\n').pop()?.trim();
  if (!alias) throw new Error(`Could not calculate the alias of ${nickname}`);
  return alias;
}
