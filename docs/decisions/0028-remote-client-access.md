# ADR 0028: Remote client access

Reader: developers and node owners.

## Status
Accepted

## Related
- ADR-0024 (client gateway identity modes and the node auth service). This ADR
  does not change it. ADR-0024 says the browser and the gateway are on the same
  machine and that untrusted-network browser access is out of scope. This ADR
  makes a remote client a supported setup, with the rules below.
- ADR-0016 (caller identity on native calls)

## Context
The client gateway is the node's HTTP entry point. It reads the first request
of a TCP connection, picks the target service from the `Host` header, and then
copies the bytes of the connection to that service. It signs every stream with
the node key.

Until now the gateway bound `0.0.0.0` and had no rule about which machine may
connect. The Roym spec says it binds `127.0.0.1`. A person who runs a node on a
home server cannot use the Hub from a laptop without a safe way to allow it.

The `local` login method has no proof step. The node reads a key file from
`person_identities_dir` itself. It is safe only for a program on the same
machine.

## Decision
1. **Remote access is a setting, off by default.** The optional table
   `[roles.client_gateway.remote_access]` has one required field,
   `bind_address` (an IP address; the port stays `http_port`). When the table is
   absent, the gateway binds `127.0.0.1` and drops at accept any TCP peer that
   is not a loopback address. When it is present, the gateway binds
   `bind_address`. A table that is spelled wrongly leaves remote access off.
2. **`local` login has three checks.** It works only for a program on this
   machine, so:
   - With remote access off, the gateway binds loopback (check 1).
   - The auth service answers `local` only when the verified caller of the
     stream is the node's own DID (check 2). A stream from another node over
     Iroh is refused. Before this ADR, the auth service ignored the caller, so
     another node could ask for a `local` token.
   - The auth service refuses `local` when the request carries `Forwarded`,
     `X-Forwarded-For`, `X-Real-IP` or `Via` (check 3). A header never allows a
     login. This check catches some proxies only. Caddy, Traefik and Apache
     `mod_proxy` add `X-Forwarded-For` by default. nginx `proxy_pass` and
     HAProxy add it only when configured. `ssh -L`, `socat` and `kubectl
     port-forward` add no header. No document relies on check 3.
   - The existing refusal of a request with an `Origin` header stays.
3. **`local` is off when remote access is on.** `GET .../methods` lists only
   `delegated-key`. Every `local` refusal names `delegated-key`. Remote clients
   use `delegated-key`, which has a challenge and a signature.
4. **Any program that forwards connections to the gateway needs remote access
   on.** This covers a TLS proxy, an SSH tunnel and a port forwarder, even when
   `bind_address = "127.0.0.1"`. Otherwise every request reaches the gateway from
   loopback and looks local.
5. **`fixed` identity mode is refused with remote access.** Startup fails with
   an error that names both settings. Every network client would act as the
   fixed person.
6. **`open` mode, or `login` mode without `connection_auth_gate`, with remote
   access:** startup succeeds with a warning. In these modes a request with no
   session reaches its target as this node's DID, self-asserted. With remote
   access, every client on the network can act as this node, on this node and on
   other nodes the gateway reaches. A personal node should use `login` with
   `connection_auth_gate = true`. The public community node uses `open` on
   purpose.
7. **Plain HTTP warning.** The gateway has no TLS. A non-loopback
   `bind_address` logs a warning. A remote Hub needs HTTPS: a browser offers
   `crypto.subtle`, which the Hub needs to sign in, only in a secure context.
8. **CORS pattern.** An `allowed_origins` entry of the form
   `<scheme>://*.<domain>[:<port>]` matches every host under the domain with
   the same scheme and port. Hub host names carry hashes, so exact entries do
   not scale. The pattern admits any page that the gateway serves under that
   domain, including other people's services that it resolves from the
   registry, to call the auth service with credentials. This is safe only while
   no client logs in with credentials: the Hub's auth requests send no
   `credentials` option, so the browser never stores the auth cookie from them.
9. **The Hub narrows its own CSP.** The static page allows `connect-src
   'self' http: https:`. A small module adds a second policy tag, built from
   the same function that decides where the Hub logs in. A browser enforces
   both, so the Hub may call itself and exactly one auth origin.
10. **Docker.** The community node's example config turns remote access on with
    `bind_address = "0.0.0.0"`, because a gateway bound to loopback cannot be
    reached through a container port mapping.

## Consequences
- A node that ran with its own config and no `remote_access` table becomes
  local-only after the upgrade. Its gateway stops answering other machines. The
  startup log says so. The owner adds the table to restore the old behaviour.
- A remote Hub needs HTTPS, an `allowed_origins` entry for its address, and
  `secure_cookies = true`. The TLS proxy needs a certificate for `*.<domain>`
  and `auth.<domain>`.
- Check 2 rests on one fact: only the gateway sends HTTP bytes signed by the
  node key. The outbound proxy also signs native calls with the node key when a
  call has no service id, but those carry a JSON-RPC preamble, so they never
  reach the auth service's HTTP handler. If the outbound proxy ever gains HTTP
  calls signed by the node key, check 2 must be revisited.
- In `fixed` mode a `local` login through the gateway is refused, because every
  stream carries the fixed person as caller. So is a `local` login on a stream
  that also carries a valid session token.
- `local` stays a same-machine convenience with no proof step.

## Alternatives considered
- **The gateway adds the peer address to the route preamble**, and the router
  passes it to the auth service, trusted only from the node's own key. More
  exact, but it changes the preamble, the SDK passthrough, the router and the
  caller context. Check 1 already guarantees a loopback peer when remote access
  is off.
- **The router sends the CSP as a response header**, from a policy the app
  manifest declares. It would also allow `frame-ancestors`. It needs a new
  manifest field, router code and deploy validation, for a rule the Hub can
  already narrow itself.
