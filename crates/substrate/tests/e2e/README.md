# Substrate WebRTC Browser E2E Tests

This directory contains a black-box, end-to-end browser automation suite built using **Playwright** and **TypeScript** to verify that a client browser can successfully access mini-apps over WebRTC and fallback transport protocols.

## Test Architecture

The E2E test runs against a fully local, self-contained instance of the Syneroym ecosystem to guarantee reproducibility and speed. During the global setup:

```
┌────────────────────────────────────────────────────────────────┐
│                   Playwright E2E Environment                   │
│                                                                │
│  ┌───────────────────────┐          ┌───────────────────────┐  │
│  │   syneroym-substrate  │          │   miniapp-demo1-web   │  │
│  │   (Substrate Daemon)  │◄─────────│   (Test Rust Backend) │  │
│  └───────────────────────┘          └───────────────────────┘  │
│              ▲                                                 │
│              │ (WebRTC Signalling & Data Plane)                │
│              ▼                                                 │
│  ┌───────────────────────┐                                     │
│  │    Chromium Browser   │                                     │
│  │  (with Service Worker)│                                     │
│  └───────────────────────┘                                     │
└────────────────────────────────────────────────────────────────┘
```

1. **Clean Workspace Initialization:** Creates a temporary, isolated config directory (`.e2e-data`).
2. **Infrastructure Initialization:** Generates a local node identity and sets up a local Substrate config.
3. **Local Relays & Registries Boot:**
   * Runs the **Community Registry** HTTP server (port `7661`).
   * Runs the local **Iroh QUIC Relay** (port `7664`) and signalling server.
   * Runs the local **WebRTC Signalling** WebSocket server (port `7663`) and Bootstrap HTTP server (port `7662`).
   * Runs the **Client Gateway** reverse proxy (port `7660`).
4. **Substrate Daemon Spin-up:** Spawns a background `syneroym-substrate` node configured with local relays and communication interfaces.
5. **Mini-app Backend Boot:** Launches `miniapp-demo1-web` (listening on port `3000`).
6. **E2E Deployment:** Registers the mini-app with the local registry and deploys the TCP passthrough routing.

## Test Scenarios Covered

The test suite in [`tests/webrtc.spec.ts`](tests/webrtc.spec.ts) acts as a real client using the WebRTC proxying service worker:

1. **Bootstrap & Navigation:** Loads the bootstrap page via the registered app alias, activates the proxying Service Worker, and verifies successful rendering of the home page.
2. **REST API Interactions (POST/GET):** Intercepts and proxies dynamic API comments (`POST /api/comments`) over the WebRTC DataChannel.
3. **Real-time WebSockets:** Connects to the local WebSocket echo/broadcast endpoint and asserts that messages are successfully signaled live.
4. **Binary File Handling:** Intercepts and verifies file uploads (`POST /api/files`) and dynamic downloads (`GET /api/files/:filename`) over WebRTC.

## How to Run Tests

### Workspace level (Recommended)
From the root of the Syneroym workspace:

```bash
# Run all tests (Rust + Playwright E2E)
mise run test:all

# Run E2E tests only (headless)
mise run test:e2e

# Run E2E tests in interactive UI Mode (with UI visible)
mise run test:e2e-ui
```

### Local level
From this directory:

```bash
# Install Node dependencies
npm install

# Run the test suite (headless)
npm test

# Run tests in Playwright's interactive UI Mode (highly recommended for debugging)
npx playwright test --ui

# Run tests in headed browser mode
npx playwright test --headed

# Run tests in Playwright Inspector (step-by-step debug mode)
npm run test:debug
```
