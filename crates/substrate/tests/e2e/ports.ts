import * as dgram from 'dgram';
import * as fs from 'fs';
import * as net from 'net';
import * as path from 'path';

/**
 * Represents a port held open by a probe socket until the daemon is ready to bind.
 *
 * Check-then-use gap nuance: The probe socket keeps the port bound while configs
 * are written, and releases it immediately before process spawn. This minimizes
 * the check-then-use window to seconds on the process-spawn path (while child
 * daemons boot and reach their bind calls), though it is not strictly zero until
 * the substrate supports native `:0` binding.
 */
export interface ReservedPort {
  port: number;
  release: () => Promise<void>;
}

// Ports below 32_768 sit outside the OS ephemeral range (Linux defaults to
// 32768-60999, as on GitHub Actions runners), so the kernel never gives one to
// an outbound connection on its own. A port from `listen(0)` is always inside
// the ephemeral range. In the gap between release and the daemon's bind, an
// outbound connection of the daemon itself (for example iroh dialling its
// relay) can take that port, and the bind then fails with EADDRINUSE. This
// pool matches `PORT_POOL_*` in `crates/substrate/tests/common/mod.rs`.
const POOL_START = 18_000;
const POOL_END = 32_768;
const MAX_PROBE_ATTEMPTS = 500;

function randomPoolPort(): number {
  return POOL_START + Math.floor(Math.random() * (POOL_END - POOL_START));
}

// Try random pool ports until one binds. Only "port is taken" errors are
// retried; any other error is a real failure.
async function reserveInPool(
  tryReserve: (port: number) => Promise<ReservedPort>,
): Promise<ReservedPort> {
  for (let attempt = 0; attempt < MAX_PROBE_ATTEMPTS; attempt++) {
    try {
      return await tryReserve(randomPoolPort());
    } catch (err) {
      const code = (err as NodeJS.ErrnoException).code;
      if (code !== 'EADDRINUSE' && code !== 'EACCES') throw err;
    }
  }
  throw new Error(
    `no free port in ${POOL_START}-${POOL_END - 1} after ${MAX_PROBE_ATTEMPTS} attempts`,
  );
}

export function reserveTcpPort(host = '127.0.0.1'): Promise<ReservedPort> {
  return reserveInPool(
    (port) =>
      new Promise((resolve, reject) => {
        const server = net.createServer();
        server.unref();
        server.once('error', reject);
        server.listen(port, host, () => {
          resolve({
            port,
            release: () => new Promise<void>((res) => server.close(() => res())),
          });
        });
      }),
  );
}

export function reserveUdpPort(host = '127.0.0.1'): Promise<ReservedPort> {
  return reserveInPool(
    (port) =>
      new Promise((resolve, reject) => {
        const socket = dgram.createSocket('udp4');
        socket.unref();
        socket.once('error', (err) => {
          try {
            socket.close();
          } catch {
            // Already closed: nothing left to free.
          }
          reject(err);
        });
        socket.bind(port, host, () => {
          resolve({
            port,
            release: () => new Promise<void>((res) => socket.close(() => res())),
          });
        });
      }),
  );
}

export interface E2EPorts {
  gatewayPort: number;
  registryPort: number;
  irohHttpPort: number;
  irohQuicPort: number;
  webrtcSignalingPort: number;
  webrtcBootstrapPort: number;
  miniappPort: number;
}

export interface MultihopPorts {
  cRegistryPort: number;
  cIrohHttpPort: number;
  cIrohQuicPort: number;
  cWebrtcSigPort: number;
  cWebrtcBootPort: number;
  cGatewayPort: number;
  cpIrohHttpPort: number;
  cpIrohQuicPort: number;
  cpWebrtcSigPort: number;
  cpWebrtcBootPort: number;
  miniappPort: number;
}

function resolvePortsFilePath(dataDir: string): string {
  const directPath = path.join(process.cwd(), dataDir, 'ports.json');
  if (fs.existsSync(directPath)) {
    return directPath;
  }
  const relPath = path.join(process.cwd(), 'crates/substrate/tests/e2e', dataDir, 'ports.json');
  if (fs.existsSync(relPath)) {
    return relPath;
  }
  return path.join(__dirname, dataDir, 'ports.json');
}

export function readE2EPorts(dataDir = '.e2e-data'): E2EPorts {
  const filePath = resolvePortsFilePath(dataDir);
  const data = fs.readFileSync(filePath, 'utf8');
  return JSON.parse(data);
}

export function readMultihopPorts(dataDir = '.e2e-data-multihop'): MultihopPorts {
  const filePath = resolvePortsFilePath(dataDir);
  const data = fs.readFileSync(filePath, 'utf8');
  return JSON.parse(data);
}
