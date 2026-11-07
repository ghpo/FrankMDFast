# Security policy

This repository is a **fork** of
[akitaonrails/FrankMDFast](https://github.com/akitaonrails/FrankMDFast). Security
reports about code that exists upstream should go to the upstream project.

## Scope

The remote-access feature added by this fork (`frankmd-remote`, the embedded web
server/SPA, Settings › Remote) is in scope here. It is opt-in:

- The server binds to the machine's Tailscale address by default (loopback is
  also allowed); wildcard/LAN binds require the explicit `--serve-bind-any`.
- A token of at least 32 bytes is mandatory; there is no unauthenticated mode.
- Traffic is plain HTTP **inside** the Tailscale/WireGuard tunnel. Never expose
  the port directly to the public internet; do not run `--serve-bind-any` on an
  untrusted network.
- Tokens belong in `FRANKMD_AUTH_TOKEN`, `.fed` (`auth_token`) or the global
  config; `--serve-token` is visible in `ps`.

## Reporting a vulnerability

Please use **GitHub private vulnerability reporting** on this repository
(Security tab → *Report a vulnerability*). If that is unavailable, open an issue
asking for a private contact channel **without** including exploit details.

Include the affected commit/version, reproduction steps and impact. Please do
not open a public issue for a vulnerability.
