import { execFileSync } from 'child_process';
import * as fs from 'fs';
import * as path from 'path';
import {
  GREETER_WASM, MINIAPP_WASM, MINIAPP_WASM_DIR, NodeHandle, SUBSTRATE_BIN, TEST_KEK, WORKSPACE_DIR,
  bootNode, buildArtifacts, parentOf,
} from './node-boot';

// The two nodes of the demo stories. S1 also runs the registry and the
// coordinator, so S2 needs nothing else to find it. Names follow
// docs/demo/demo.md.

export const DEMO_DIR = path.join(__dirname, '.e2e-data-demo');
export const ENV_FILE = path.join(DEMO_DIR, 'demo.env');
const LOG_DIR = path.join(__dirname, 'e2e-logs');

const GREETER_IFACE = 'syneroym-test:greeter/greet@0.1.0';

export interface DemoPids {
  S1_PID: string;
  S2_PID: string;
}

function quote(value: string): string {
  return `"${value.replace(/(["\\$`])/g, '\\$1')}"`;
}

function writeEnvFile(s1: NodeHandle, s2: NodeHandle): void {
  const vars: Record<string, string | number> = {
    REPO: WORKSPACE_DIR,
    DEMO_HOME: DEMO_DIR,
    BIN: path.dirname(SUBSTRATE_BIN),
    REGISTRY: s1.registryUrl,
    S1_DIR: s1.dir,
    S2_DIR: s2.dir,
    S1_DID: s1.did,
    S2_DID: s2.did,
    S1_GW: s1.ports.gateway!,
    S2_GW: s2.ports.gateway!,
    S1_WEBRTC_BOOT: s1.ports.webrtcBootstrap!,
    KEK: TEST_KEK,
    GREETER_WASM,
    GREETER_IFACE,
    MINIAPP_DIR: MINIAPP_WASM_DIR,
    MINIAPP_WASM,
    S1_PID: s1.process.pid!,
    S2_PID: s2.process.pid!,
  };
  const lines = Object.entries(vars).map(([k, v]) => `export ${k}=${quote(String(v))}`);
  // Write a whole file and rename it, so a reader never sees half of it.
  const tmp = `${ENV_FILE}.tmp`;
  fs.writeFileSync(tmp, lines.join('\n') + '\n');
  fs.renameSync(tmp, ENV_FILE);
}

/** Read the env file. Story scripts append to it, so read it again when you need a fresh value. */
export function readDemoEnv(): Record<string, string> {
  if (!fs.existsSync(ENV_FILE)) {
    throw new Error(`No demo env file at ${ENV_FILE}. Run "mise run demo:up" first.`);
  }
  const env: Record<string, string> = {};
  for (const line of fs.readFileSync(ENV_FILE, 'utf8').split('\n')) {
    const match = line.match(/^(?:export\s+)?([A-Za-z_][A-Za-z0-9_]*)=(.*)$/);
    if (!match) continue;
    env[match[1]] = match[2].replace(/^"(.*)"$/, '$1').replace(/\\(["\\$`])/g, '$1');
  }
  return env;
}

/**
 * Build everything the stories use, then start S1 and S2 and write the env file.
 * With `detached` the nodes keep running after this process exits.
 */
export async function bootDemoNodes(detached: boolean): Promise<{ s1: NodeHandle; s2: NodeHandle }> {
  refuseIfNodesRunning();
  if (fs.existsSync(DEMO_DIR)) fs.rmSync(DEMO_DIR, { recursive: true, force: true });
  fs.mkdirSync(DEMO_DIR, { recursive: true });

  buildArtifacts({
    miniappWasm: true,
    roym: true,
    greeter: true,
    binaries: ['roymctl', 'substrate'],
  });

  const common = {
    interfaces: ['webrtc', 'iroh'],
    claim: true,
    injectKek: true,
    detached,
    // The Roym outbox runs once per tick; the default of 5 s makes a story wait.
    appSandbox: { conversationTickSecs: 1 },
  };

  const s1Dir = path.join(DEMO_DIR, 's1');
  const s1 = await bootNode('S1', {
    ...common,
    dir: s1Dir,
    logPath: path.join(s1Dir, 'substrate.log'),
    nickname: 'alice-node',
    registry: true,
    coordinator: {},
    gateway: { login: true },
    parent: 'self',
  });

  const s2Dir = path.join(DEMO_DIR, 's2');
  let s2: NodeHandle | undefined;
  try {
    s2 = await bootNode('S2', {
      ...common,
      dir: s2Dir,
      logPath: path.join(s2Dir, 'substrate.log'),
      nickname: 'bob-node',
      gateway: { login: true },
      registryUrl: s1.registryUrl,
      parent: parentOf(s1.ports),
    });
    writeEnvFile(s1, s2);
    return { s1, s2 };
  } catch (err) {
    s1.process.kill('SIGKILL');
    s2?.process.kill('SIGKILL');
    throw err;
  }
}

/** True if the process is a demo node: our substrate binary, run on a config under the demo directory. */
function isDemoNode(pid: number): boolean {
  try {
    const command = execFileSync('ps', ['-p', String(pid), '-o', 'command='], { encoding: 'utf8' });
    return command.includes(path.basename(SUBSTRATE_BIN)) && command.includes(DEMO_DIR);
  } catch {
    return false;
  }
}

/**
 * Deleting the demo directory under nodes that are still running would leave
 * them as orphans that "demo:down" can no longer find, so stop early instead.
 */
function refuseIfNodesRunning(): void {
  if (!fs.existsSync(ENV_FILE)) return;
  const env = readDemoEnv();
  const running = [env.S1_PID, env.S2_PID].filter(pid => pid && isDemoNode(Number(pid)));
  if (running.length > 0) {
    throw new Error(
      `Demo nodes are already running (pid ${running.join(', ')}). Run "mise run demo:down" first.`);
  }
}

async function answers(url: string): Promise<boolean> {
  try {
    await fetch(url, { signal: AbortSignal.timeout(3000) });
    return true;
  } catch {
    return false;
  }
}

/** Check that the nodes `demo:up` started still answer, and return the env file. */
export async function attachToDemoNodes(): Promise<Record<string, string>> {
  const env = readDemoEnv();
  for (const pid of [env.S1_PID, env.S2_PID]) {
    if (!pid || !isDemoNode(Number(pid))) {
      throw new Error(`Demo node (pid ${pid}) is not running. Run "mise run demo:up" first.`);
    }
  }
  const checks: Array<[string, string]> = [
    ['S1 registry', env.REGISTRY],
    ['S1 gateway', `http://127.0.0.1:${env.S1_GW}/`],
    ['S2 gateway', `http://127.0.0.1:${env.S2_GW}/`],
  ];
  for (const [what, url] of checks) {
    if (!(await answers(url))) {
      throw new Error(`${what} does not answer at ${url}. Run "mise run demo:up" first.`);
    }
  }
  return env;
}

function isAlive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
}

async function stopProcess(pid: number): Promise<void> {
  if (!isAlive(pid)) return;
  // The pid may belong to another program now, for example after a reboot.
  if (!isDemoNode(pid)) {
    console.warn(`pid ${pid} is not a demo node any more; leaving it alone.`);
    return;
  }
  process.kill(pid, 'SIGTERM');
  const deadline = Date.now() + 5000;
  while (isAlive(pid) && Date.now() < deadline) {
    await new Promise(r => setTimeout(r, 100));
  }
  if (isAlive(pid)) process.kill(pid, 'SIGKILL');
}

/** Stop both nodes, keep their logs under `e2e-logs/`, and remove the demo data. */
export async function stopDemoNodes(pids: DemoPids): Promise<void> {
  await Promise.all([stopProcess(Number(pids.S1_PID)), stopProcess(Number(pids.S2_PID))]);
  fs.mkdirSync(LOG_DIR, { recursive: true });
  for (const node of ['s1', 's2']) {
    const log = path.join(DEMO_DIR, node, 'substrate.log');
    if (fs.existsSync(log)) fs.copyFileSync(log, path.join(LOG_DIR, `demo-${node}.log`));
  }
  fs.rmSync(DEMO_DIR, { recursive: true, force: true });
}
