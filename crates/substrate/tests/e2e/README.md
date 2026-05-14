## E2E browser automation test for webrtc
This is an end to end test to verify that miniapps can be accessed via webrtc. Uses playwright for automation. We spin up substrate, a web miniapp and then use playwright to exercise various features of the miniapp.

This is work in progress. The automation is still not working as expected. The below instructions show how the automation can be run step by step for manual troubleshooting. Remember, there may be some issue in the test steps too. 

### Troubleshooting the "Timed out waiting for substrate to become ready" error
The `roymctl app deploy` command is currently failing in automation because the `SyneroymClient` (used internally by `roymctl`) cannot connect to the local substrate via Iroh. This usually happens if:
1. The substrate hasn't fully registered its Iroh endpoint in the Community Registry yet.
2. There's a connectivity issue with the local Iroh node (e.g., port binding or relay discovery).

### Manual Troubleshooting Guide
If you'd like to troubleshoot this manually, follow these steps to set up the environment yourself:

#### 1. Initialize a Test Directory
```bash
mkdir -p .e2e-data
# Initialize a node identity
cargo run --bin roymctl -- node init --dir .e2e-data
```

#### 2. Start the Substrate (Terminal 1)
Create a `syneroym.toml` in `.e2e-data` with the ports we defined (7660-7665) and set `registry_url = "http://127.0.0.1:7661"`.
```bash
cargo run --bin syneroym-substrate -- run --config .e2e-data/syneroym.toml
```
*Look for the line: `substrate identity initialized, did: did:key:h...` and copy the DID.*

#### 3. Start the Miniapp (Terminal 2)
```bash
cargo run -p miniapp-demo1-web -- --port 3000 --data-dir .e2e-data/miniapp-data
```

#### 4. Deploy and Register (Terminal 3)
```bash
# 1. Create an identity for the app
cargo run --bin roymctl -- identity create --name demo1 --dir .e2e-data
# Get the App DID
APP_DID=$(cargo run --bin roymctl -- identity show --name demo1 --dir .e2e-data | grep -o 'did:key:[a-z0-9]*')

# 2. Register the app in the Registry
cargo run --bin roymctl -- registry register --identity demo1 --substrate <SUBSTRATE_DID> --nickname demo1 --dir .e2e-data --api-url http://127.0.0.1:7661

# 3. Deploy the TCP passthrough
cargo run --bin roymctl -- app deploy --app-id $APP_DID --interfaces http --tcp 127.0.0.1:3000 --substrate <SUBSTRATE_DID> --dir .e2e-data --api-url http://127.0.0.1:7661
```

#### 5. Verify in Browser
Calculate the alias:
```bash
cargo run --bin roymctl -- alias $APP_DID --nickname demo1 --interface http
```
Visit the resulting URL on port **7662** (e.g., `http://demo1-p9y1qiex4-ihboda1c4.localhost:7662/`).

---

I have documented the progress in the [task.md](file:///Users/pari/.gemini/antigravity/brain/6d515352-0d6d-404d-a3a7-5a79f4e55f84/task.md) and provided the updated code for the tests.

```typescript
// crates/substrate/tests/e2e/tests/webrtc.spec.ts
const appAlias = process.env.APP_ALIAS;
const url = `http://${appAlias}:7662/`; // Corrected URL
await page.goto(url);
```
