import * as dgram from 'dgram';
import * as fs from 'fs';
import * as net from 'net';
import * as path from 'path';

/**
 * Represents a port held open by a probe socket until the daemon is ready to bind.
 *
 * Check-then-use gap nuance: The probe socket keeps the port bound while configs
 * are written, and releases it immediately before process spawn. This minimizes
 * the check-then-use window to milliseconds, though it is not strictly zero until
 * the substrate supports native `:0` binding.
 */
export interface ReservedPort {
  port: number;
  release: () => Promise<void>;
}

export function reserveTcpPort(host = '127.0.0.1'): Promise<ReservedPort> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on('error', reject);
    server.listen(0, host, () => {
      const addr = server.address() as net.AddressInfo;
      const port = addr.port;
      resolve({
        port,
        release: () =>
          new Promise<void>((res) => {
            server.close(() => res());
          }),
      });
    });
  });
}

export function reserveUdpPort(host = '127.0.0.1'): Promise<ReservedPort> {
  return new Promise((resolve, reject) => {
    const socket = dgram.createSocket('udp4');
    socket.unref();
    socket.on('error', reject);
    socket.bind(0, host, () => {
      const addr = socket.address();
      const port = addr.port;
      resolve({
        port,
        release: () =>
          new Promise<void>((res) => {
            socket.close(() => res());
          }),
      });
    });
  });
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
