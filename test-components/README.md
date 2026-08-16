# Test Components

This directory contains various applications and components used for integration and end-to-end testing of parts of Syneroym.

## Components

- **[greeter](greeter/)**: A simple WASM component used to verify basic RPC and interface binding functionality.
- **[miniapp-demo1-web](miniapp-demo1-web/)**: A comprehensive web application demo testing static/dynamic content, REST APIs, large file transfers, and real-time WebSockets in a P2P environment.
- **[http-guest-test](http-guest-test/)**: Exercises the M06A A2 guest HTTP route target (`syneroym:http/incoming-handler#handle-request`) -- echoes request shape, caller identity, and exercises the reject/fail/trap/spin/oversized-response/malformed-header/concurrency failure paths.
- **[websocket-guest-test](websocket-guest-test/)**: Exercises the M06A A3 inbound WebSocket route target (`syneroym:http/websocket-handler`).
- **[miniapp-demo1-wasm](miniapp-demo1-wasm/)**: A pure WASM component providing the same web application functionality as `miniapp-demo1-web` (M06A A4): static asset serving via A1, REST endpoints via A2, upload/download streams, and live updates over both WebSocket (A3) and SSE (M3B).
