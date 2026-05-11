# Syneroym API Examples

This document provides examples of how to interact with the Syneroym substrate using standard tools like `curl`.

## Port Reference (Normalized 796x)

- **7960**: Client Gateway (HTTP Proxy)
- **7961**: Community Registry (HTTP)
- **7962**: WebRTC Bootstrap Page (HTTP)
- **7963**: WebRTC Signaling Server (WebSocket)
- **7964**: Iroh Coordinator (HTTP Signaling)
- **7965**: Iroh Coordinator (QUIC Data)

---

## 1. Discovering Services

### Lookup a specific service by its DID
```bash
# Returns signed endpoint info
curl http://localhost:7961/lookup/did:key:z6MkhaXn...
```

---

## 2. Managing Applications (Orchestrator)

The Orchestrator is a native service running inside the substrate. You can interact with it via the Client Gateway (Port 7960).

### List Deployed Services
```bash
# Replace <SUBSTRATE_DID_HASH> with the short hash of your substrate's DID
curl -X POST http://localhost:7960/ \
  -H "Host: orchestrator-p<SUBSTRATE_DID_HASH>.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "list",
    "params": {},
    "id": 1
  }'
```

### Deploy a WASM Component
```bash
# Note: WASM binary bytes are usually sent as a base64-encoded array or via a URL.
curl -X POST http://localhost:7960/ \
  -H "Host: orchestrator-p<SUBSTRATE_DID_HASH>.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "deploy",
    "params": [
      "did:key:my-app-did",
      ["my-interface:v1"],
      {
        "config": { "env": [], "args": [], "custom_config": null },
        "service_type": {
          "wasm": {
            "source": { "url": "http://example.com/app.wasm" },
            "hash": "sha256:..."
          }
        }
      }
    ],
    "id": 1
  }'
```

### Deploy a TCP Service (Passthrough)
```bash
curl -X POST http://localhost:7960/ \
  -H "Host: orchestrator-p<SUBSTRATE_DID_HASH>.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "deploy",
    "params": [
      "did:key:my-tcp-service",
      ["default"],
      {
        "config": { "env": [], "args": [], "custom_config": null },
        "service_type": {
          "tcp": {
            "host": "localhost",
            "port": 8080
          }
        }
      }
    ],
    "id": 1
  }'
```

---

## 3. Interacting with Applications

### Call a JSON-RPC method on a WASM app via HTTP Proxy
```bash
# Host header format: <NICKNAME>-p<APP_DID_HASH>-i<INTERFACE_HASH>.localhost
curl -X POST http://localhost:7960/ \
  -H "Host: my-app-p<APP_DID_HASH>-i<INTERFACE_HASH>.localhost" \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "greet",
    "params": ["Syneroym User"],
    "id": 1
  }'
```

### Call a TCP service via HTTP Proxy
```bash
# Simple GET request
curl http://localhost:7960/api/data \
  -H "Host: my-tcp-service-p<APP_DID_HASH>-i<INTERFACE_HASH>.localhost"
```

---

## 4. Health and Metrics

### Health Check
```bash
curl http://localhost:7966/health
```

### Prometheus Metrics
```bash
curl http://localhost:7967/metrics
```
