---
status: draft
living-docs-touched:
demo:
  - crates/substrate/tests/e2e/demo/10-substrate-main.spec.ts
  - crates/substrate/tests/e2e/demo/20-roym-chat.spec.ts
---
# Demo stories

*Reader: developers and the project owner, when they plan or build a slice that should come with a demo.*

## Problem

Nobody can see Syneroym or Roym work from end to end without reading tests. The only demo script, [docs/demo/demo.md](../../../demo/demo.md), is a manual runbook of about 1,400 lines. It was never run from start to finish, so it is probably out of date.

Milestone 8 adds many slices. We want a demo of the full product to be ready after each slice, without a manual script that drifts.

## Scope and non-goals

- In scope:
  - Demo stories for the substrate (deploy, call, web apps, and so on) and for Roym (sign in, chat, and so on).
  - Two kinds of story: shell scripts for terminal work, Playwright specs for browser work.
  - One harness that starts **two** local nodes, used by both kinds.
  - A shared node-boot library for the main suite and the demo, and for the multihop suite unless a deviation says otherwise, so the boot code exists once.
  - A way to record a video of a browser story, only when asked, to a local folder.
  - A rule, checked by a tool, that every change doc says what demo it adds or why it has none.
- Not in scope:
  - Failure cases. A demo shows the working path only. The existing tests cover failures.
  - Storing videos or screenshots in git or in a PR.
  - The cloud machine and real network paths (relay on another host, UDP port, nip.io). For now all nodes run on this computer over loopback. `docs/demo/demo.md` keeps the cloud steps and its known traps unchanged.
  - Stories for features that Milestone 8 is about to rebuild (see "What the first stories cover").

## Naming

The demo uses the runbook's names, so scripts and runbook read the same: **S1** and **S2** are the two nodes, and `rc1` and `rc2` run `roymctl` against them. In the Roym story, Alice is the person on S1 and Bob is the person on S2. S1 also runs the registry and the coordinator. The env file sets `REGISTRY`, `S1_DIR`, `S1_DID`, `S1_GW` and so on, with dynamic ports. It never uses 7970 or 7980.

## Acceptance criteria

| ID | Criterion | Test |
| --- | --- | --- |
| DM-1 | `mise run demo` boots S1 and S2, runs every story in order, then stops both nodes. `mise run demo:up` boots them and leaves them running with the env file written. `mise run demo:down` stops them. | manual check, in the task list |
| DM-2 | With `DEMO_VIDEO=1`, each **browser** page is recorded to `crates/substrate/tests/e2e/demo-output/`. A step that needs two sessions gives two videos. Shell steps are not recorded. Without `DEMO_VIDEO=1` nothing is recorded. | manual check, in the task list |
| DM-3 | Substrate main story: a small service is deployed on S1 and called on S1. The same service is called through S2 with a JSON-RPC call. A web app is deployed on S1, opened in the browser, and a change made in one session shows live in a second session. | `demo/10-substrate-main.spec.ts` |
| DM-4 | Roym chat story: Alice and Bob each set a profile with their conversation address. Alice adds Bob as a contact using his DID and address. Alice sends a message. Bob reopens the conversation and sees it. Bob replies. Alice refreshes and sees her message in state `delivered`, and Bob's reply. | `demo/20-roym-chat.spec.ts` |
| DM-5 | Every step prints or shows a caption that names the step. In a Roym story the caption also says whose screen it is. | `demo/20-roym-chat.spec.ts` |
| DM-6 | Each shell story runs as a test (exit code 0), so a broken command fails the demo run. | `demo/10-substrate-main.spec.ts` |
| DM-7 | Each converted section of `docs/demo/demo.md` stays for manual use and starts with a line that names the script as the source of truth. | review |
| DM-8 | A change doc with `status: done` must have a `demo:` front-matter list. Each item is a story path that exists, or `none: <reason>`. | `cargo xtask check-change-docs` |

## Design

### The idea

A demo story is a real test that tells a story. It uses the real binaries (`syneroym-substrate`, `roymctl`) and, for Roym, the real Hub. A story is not a second copy of a test. It is a short script of the working path with captions. Because it is code, a change that breaks it is found when the demo runs.

Stories live in `crates/substrate/tests/e2e/demo/`, beside the main suite's `tests/` folder.

### Shared boot library

Three Playwright setups boot nodes today or will: `global-setup.ts` (one node, 410 lines), `global-setup-multihop.ts` (four nodes, 349 lines, run by `npm test` too) and the new demo. The build step is already copied between the first two. A third copy is not acceptable. Create `node-boot.ts` with:

- `buildArtifacts(options)`: the UI bundles, the WASM components and the Cargo binaries. The demo adds the `greeter` component, which no setup builds today but the main story needs.
- `bootNode(name, options)`: data directory, ports from `ports.ts`, identity and claim, config, spawn with a log file, wait for the node DID, and KEK injection (the current setup injects the data key after start, and the runbook requires it).

The ways nodes connect (`community_registry_url`, `parent_coordinator`) are copied from `global-setup-multihop.ts`, not from the Rust tests. Roym deploy, nickname, session keys and signing enrolment stay outside `bootNode`: the main suite keeps its own calls, and the demo shows them as shell steps because they are what the viewer should see.

Conversion order, one commit each: (1) main suite, (2) demo, (3) multihop. If multihop does not fit the options without strain, keep its boot code and record a deviation. Then also add a row to [deferred-backlog.md](../../deferred-backlog.md), because a second copy of the boot code is a known shortcut. The main suite's behaviour must not change.

### Two kinds of story, one runner

Playwright is the runner for tests. This gives one lifecycle, one order and one report.

- **Browser story:** a Playwright spec.
- **Shell story:** a script `demo/NN-name.sh`. It loads the env file and a small `lib.sh`. `lib.sh` has `caption`, and `run`, which prints a command and then runs it. It keeps the helper names of [demo.env.example](../../../demo/demo.env.example). Each story spec runs its own shell scripts first (each must exit with code 0), then its browser steps. The Roym chat spec, for example, runs `20-roym-setup.sh` and then drives the two Hubs. Shell steps that learn a value (for example a conversation address from `roymctl roym address`) append `KEY=value` to the env file. Specs read the same file.
- **Order and state.** Files run in name order (`10-`, `20-`). Stories are cumulative, as the runbook assumes: `demo:up` gives two empty nodes, the substrate main story creates the first state, and the Roym story deploys Roym on both nodes as its first step.

### Live talks

`DEMO_STEP=1` (wait for Enter after each step) cannot work inside a Playwright worker, which has no terminal. So for a live talk:

1. `mise run demo:up` boots the nodes, writes the env file and leaves them running.
2. The presenter runs the shell scripts directly, with `DEMO_STEP=1`.
3. Browser parts run with `DEMO_MODE=present DEMO_ATTACH=1` against the running nodes.
4. `mise run demo:down` stops everything.

The Playwright run (`mise run demo`) is for testing and recording.

**Attach mode.** With `DEMO_ATTACH=1`, the demo setup reads the env file that `demo:up` wrote. If the nodes in it answer, the setup skips the boot, and the teardown does not stop them. If the file is missing or a node does not answer, the run stops with a message that names `mise run demo:up`. In attach mode a story spec also skips its shell scripts, because the presenter has already run them by hand, and a second run would fail or deploy twice. The spec reads the values it needs from the env file only. If a value is missing, it stops with a message that names the script to run first. We do not make the scripts safe to run twice: that costs more work and adds checks a viewer does not need to see. Without `DEMO_ATTACH`, the setup boots its own nodes and the teardown stops them.

### Run modes

One switch: `DEMO_MODE=present` turns on headed mode, slow motion and the pause after each step (`DEMO_PACE_MS`, default 600 ms). By default a run is headless with no pause. The demo config has its own `globalTimeout`, larger than the main suite's 300 s, because two nodes boot and Roym deploys twice.

### Recording

`DEMO_VIDEO=1` sets Playwright's `video: 'on'`. Videos and traces go to `demo-output/`, which is git-ignored. Nothing else stores them. The substrate main story is mostly shell, so its video holds only the browser part. A reader of the shell part watches the terminal.

### One tab, two screens (Roym story)

Playwright records video per page, so two windows give two videos. The Roym story uses one tab. It goes to Alice's Hub address, then to Bob's, and back. A caption bar says "Alice's screen" or "Bob's screen". This gives one video. The two-session live-update step in the main story is the exception: it needs two pages.

Session isolation, by origin. The Hub session has three parts: a bearer token in `sessionStorage`, an IndexedDB store `roym-hub-session`, and a cookie with no `Domain` attribute. The first two are scoped to the origin (host and port). The cookie is scoped to the host only, not the port. The two Roym web services have different DIDs, so the two hosts differ and the cookies differ. The risk is low. Phase 0 checks it. Hub test 2 shows that a session survives a reload, so switching between the two origins keeps both sessions.

### Browser steps and the web app

The web app is opened through S1 the way the current suite does it, with the WebRTC bootstrap page. That path is proven. "Through the other node" is shown with the JSON-RPC call through S2 (runbook 2.4). The runbook marks the browser through the S2 gateway as unrehearsed, and no browser test uses a gateway host name for the web app. Trying it is an optional Phase 0 check.

### Roym chat: what it needs

The path is known from the Rust test `roym_conversation_e2e.rs`:

1. Each person's `profile.set` carries a `conversation_address`. The value comes from `roymctl roym address`. The setup step writes each address into the env file.
2. Alice adds Bob as a contact with his DID **and** his address. The Hub contact form has both fields.
3. Alice sends. The message state goes `pending`, then `deposited` or `delivered` (the states are `pending | deposited | delivered | failed`). The demo has no mailbox, so the target is `delivered`.
4. Delivery waits for the sender's outbox tick. The default `conversation_tick_secs` is 5. The demo node config sets it to 1 with only this block. Every other field keeps its default, because `AppSandboxRole` uses `#[serde(default)]`:

   ```toml
   [roles.app_sandbox]
   conversation_tick_secs = 1
   ```

   The step still waits up to 60 s. The Rust test also sets 1 s.
5. The Hub does not update live: the messages screen fetches again only after a click, with no polling and no `EventSource`. So the story has explicit steps: Bob reopens the conversation, and Alice refreshes to see `delivered`.

Hub steps use class selectors, not visible text, because M08A and M08B change the text. Shared Hub steps go into the existing `hub-helpers.ts`, not a new file. Its `loginWithDelegatedKey` takes the session-key file as a parameter, so it works for Alice and for Bob, and `roym-hub.spec.ts` and the demo get a rename fix from one place.

### What the first stories cover

Pilot 1 is the substrate main story. Pilot 2 is the Roym chat story.

| Part | Changes in M8? | In the first build |
| --- | --- | --- |
| Substrate main story (deploy, call, call through the other node, web app) | Little. M08A adds host features but does not change these steps. | **Pilot 1** |
| Roym sign in, profile, contacts, chat between two people | M08A: the "participant" rename, remote clients (a new login path and a changed CSP), and the event feed (live updates replace the refresh steps; this part gets better). M08B: Hub strings. M08C: message forwarding adds a forward-mode choice when sending. The core path stays. | **Pilot 2** |
| Other substrate stories (identity, data protection, health and metrics) | No | Later, one at a time |
| Substrate stories that need more than two nodes (multi-hop, supervisor) | No | Later |
| Roym directory search and publish | M08F splits the directory into membership and exchange. | No. Add a new story in M08F |
| Roym request, quote, accept | M08C rebuilds it as commitments and agreements. | No. Add a new story in M08C |
| Roym listings | M08D reshapes them as intents and a shop. | No. Add a new story in M08D |

Most of the effort is the harness, which every later story reuses. The Roym story will need small edits at M08A and M08C. That is expected, and the demo run finds them.

### Converting the runbook

`docs/demo/demo.md` already has the steps for the substrate main story and for stories F1 to F13. Converting a story means turning its copy-paste blocks into a script, running it, and fixing what was wrong (the runbook marks many steps "Rehearse"). Fix the script and the runbook together. A converted section stays in `demo.md` for people who want to do the steps by hand. Put one line at the top of it: "Automated: `mise run demo`. The script is the source of truth. This text may lag." When the two differ, the script wins.

### Does a demo run in `mise run verify`?

The reason to write demos as tests is that they cannot go stale. That works only if they run regularly. Booting two nodes and deploying twice adds time to a gate that already takes about 20 minutes. Phase 0 measures the added time.

- Under three minutes: add `playwright test -c playwright-demo.config.ts` to `npm test`. It then runs in `mise run verify` and in the CI job "Playwright E2E".
- Over three minutes: add an `npm run test:demo` script and a separate step "Run demo stories" after "Run Playwright E2E" in that CI job, and keep it out of `npm test`. Record the choice here.

### Enforcing the demo rule

AGENTS.md says a rule nobody checks is how drift happened. So the demo rule is a check. Add a front-matter list `demo:` to the change-doc template. `cargo xtask check-change-docs` requires it when `status: done`, in the same way as `living-docs-touched`. Each item is a story path that exists, or `none: <reason>`. The list form keeps the front matter valid YAML, because a one-line `demo: none: reason` has a second colon:

```yaml
demo:
  - none: before demo stories
```

The check works per change doc, which may cover several slices, so it is coarse. The four existing change docs that are `done` get the value above.

### Link between slice plans, the demo and AGENTS.md

A slice plan says what the demo will show. The demo files say how. The two must not repeat each other's steps, or they drift.

- **Slice plan** (each M08 `task.md` and slice implementation plan): one `Demo` line. It says in one or two sentences what a viewer will see after the slice, and names the story it adds or extends. Or it says `no demo: internal` with the reason. It does not list steps.
- **Story file:** the steps, as code. This is the source of truth.
- **Change doc:** the `demo:` list at `status: done` is the checked pointer from the plan to the story.
- **Done means:** the slice's story step runs green.
- **AGENTS.md:** gets a "Mandatory Demo Close-out" bullet in the AI Agent Guidelines, next to the living-docs close-out. It asks: *does this change add something a viewer can see?* If yes, add or extend a story and run it. Then set `demo:` in the change doc. If no, write `none: <reason>`. *Checked by:* `cargo xtask check-change-docs` for the field, and the demo run in CI. The bullet is added in the same change as the xtask check, not before, because AGENTS.md does not allow a rule without a check.

### Rejected options

- **A demo in the Rust test harness, with the browser attached.** It reuses the node code, but the demo would not use the real binaries. Rejected for that reason.
- **Markdown runbooks that a tool extracts and runs.** The current runbook has per-step notes and cloud details mixed in. A script is simpler to run and to test.
- **Two browser windows side by side.** Good for a live talk, bad for one video. Left as a later option.
- **Committing videos or attaching them to PRs.** The owner wants nothing stored. Rejected.
- **A third copy of the boot code in the demo setup.** Rejected. See "Shared boot library".

## Decisions for Phase 0

These choices are made now so that Phase 0 can start. Phase 0 checks each one. If a check fails, use the fallback and write one line under Deviations.

| Question | Decision | Fallback if the check fails |
| --- | --- | --- |
| Does S2 need its own registry role? | No. S2 uses S1's registry and coordinator, as in `global-setup-multihop.ts`. | Give S2 its own registry that points to S1. |
| Do the two Hubs keep separate sessions? | Yes. See "One tab, two screens". | Use two browser contexts and two videos for the Roym story. |
| Does the demo run in `npm test`? | Decided by the measured boot time. | See "Does a demo run in `mise run verify`?". |
| Optional: can the browser open the web app through S2's gateway? | Not needed for the pilot. | Keep the JSON-RPC call through S2. |

## Tasks

- [ ] Phase 0, shared boot library and harness
  - [x] `node-boot.ts` with `buildArtifacts` and `bootNode`. Convert the main suite. It stays green.
  - [ ] `playwright-demo.config.ts`, demo setup and teardown, env file, git-ignored `demo-output/`, `demo:up` and `demo:down`.
  - [ ] Convert `global-setup-multihop.ts` to the library, or record a deviation and add a deferred-backlog row.
  - [ ] Attach mode (`DEMO_ATTACH=1`) in the demo setup and teardown.
  - [ ] Check each decision above. Use the fallback where a check fails.
  - [ ] Measure the boot time. Record whether the demo joins `npm test`.
- [ ] Phase 1, pilot 1: substrate main story
  - [ ] `demo/lib.sh` and the shell-story runner spec. Add `greeter` to the build step.
  - [ ] Scripts for the main story (runbook section 2). Fix the runbook where it was wrong.
  - [ ] Browser step: open the web app and show a live update in a second session.
  - [ ] `mise` tasks `demo`, `demo:up`, `demo:down`.
- [ ] Phase 2, pilot 2: Roym chat story
  - [ ] Roym deploy, session keys and conversation addresses as a shell step.
  - [ ] `caption` and `step` helpers. Add the file parameter to `loginWithDelegatedKey`. Add the shared Hub steps to `hub-helpers.ts`.
  - [ ] `demo/20-roym-chat.spec.ts`.
- [ ] Phase 3, rules and docs
  - [ ] Add the `demo:` list to the change-doc template, check it in `xtask` (including that a listed path exists), and fill the four existing done docs.
  - [ ] Add the "Mandatory Demo Close-out" bullet to `AGENTS.md` in the same change as the xtask check.
  - [ ] Add a `Demo` line to each M08 `task.md`.
  - [ ] Add the "Automated" line to each converted section of `docs/demo/demo.md`.
  - [ ] `docs/developer-guide.md`: how to run and write a demo.
- [ ] Later: more substrate stories one at a time, and a story for each M8 slice that has something to show.

## Deviations

None yet.

## Close-out

Before merge, update the living docs (requirements, architecture, user guide)
so they describe the result. Then set `status: done` and fill in
`living-docs-touched`. Record anything you postponed in
[deferred-backlog.md](../../deferred-backlog.md).
