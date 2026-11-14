# ADR 0029: Machine translation host interface

Reader: developers building services or clients on Syneroym, and operators running substrates.

## Status
Accepted

## Related
- ADR-0027 (host event feed): the last host interface that was added. This one is wired the same way.
- ADR-0024 (client gateway and auth service session tokens)

## Context
A Roym person may read a record or a message in a language they do not read. A machine translation engine can help. The engine will come later, for example a local model on the node, or a remote service that the node owner chooses.

We want the places that use translation to exist and be tested before an engine exists. We also want rules about engines that hold for every engine: where the text may go, what the reader sees, and what is never signed.

## Decision
1. **A host interface, `syneroym:translation@0.1.0`.** It has three calls:
   - `supported-targets`: the languages the engine can translate into. It is empty when the node has no engine.
   - `detect-language`: the likely languages of a text, each with a confidence from 0 to 1000.
   - `translate`: a text, an optional source language and a target language. It returns the translated text, the source language that was used, and the engine.
   A call can fail with `unavailable`, `unsupported-pair`, `too-long` or `engine-failed`.
2. **The host owns the engine.** A guest cannot choose an engine, and a guest cannot name a remote one. A guest only calls the interface.
3. **A remote engine needs the node owner.** An engine that sends text off the node is used only when the node owner names it in the substrate configuration. A running service, a guest or a stranger cannot turn it on.
4. **Every result names its engine.** A `translated` result carries `engine.name` and `engine.remote`. `remote` is true when the text left this node. A client shows both next to every translation.
5. **A translation is for display only.** No interface takes a translation as input to signing. A signed record carries the text its author wrote. The original is what binds. A client that shows a translation says so, and warns before a person accepts terms that they read only through machine translation.
6. **Version 1 has no engine.** The only implementation answers `unavailable` to `detect-language` and `translate`, and an empty list to `supported-targets`. There is no configuration key that names an engine. So in version 1 no text leaves the node.
7. **No cost limit and no rate limit in version 1.** The interface has none, because the version 1 host does no work. A later engine must add both a cost limit and a rate limit before the node owner can name a remote engine.
8. **One implementation, two builds.** A Roym service runs as a `wasm32-wasip2` component or linked into the substrate. Both builds call the same host functions, so they cannot give different answers.
9. **The Hub reaches the interface through the `web` service.** `web` answers `translation.targets` and `translation.translate` itself, for the signed-in owner only, on the HTTP path only. The Hub asks for the list of target languages once after sign-in, and draws a translate action only when the list is not empty.

## Consequences
- With the version 1 node, an end user sees no translate action anywhere. The hook points are built and tested with a fake engine in unit tests.
- Adding an engine later changes one host implementation. It does not change the WIT package, the `web` methods or the Hub.
- A substrate built before this decision cannot run a Roym component built after it: the component imports an interface that the older host does not provide. Roym and the substrate are deployed together.
- A later engine has to define its own cost limit, rate limit and configuration key. This ADR sets only the rules that hold for all of them.

## Alternatives considered
- **A service chooses its engine.** Rejected: a service could then send a person's text to a remote service that the node owner never agreed to.
- **Translate inside each service, with no host interface.** Rejected: each service would carry an engine or call a remote one itself, and the node owner could not see or control where text goes.
- **A Hub-only translation, in the browser.** Rejected: the Hub would send text to a remote service from the person's browser with no control by the node owner, and `roymctl` could not use it.
- **Drop `detect-language` until a caller exists.** Rejected: adding a function later changes the package version, and an engine needs it to label text that carries no language tag.
