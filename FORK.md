# This is a fork

This repository is a fork of
[akitaonrails/FrankMDFast](https://github.com/akitaonrails/FrankMDFast),
created and maintained upstream by **Fabio Akita and the FrankMD contributors**.

- **Upstream is the original work.** This fork is not affiliated with, sponsored
  by, or endorsed by the upstream author.
- **License.** The project is MIT; the original copyright notice in
  [`LICENSE`](LICENSE) is kept unchanged. Any redistribution of binaries or
  substantial portions must include that notice.
- **What this fork adds.** Opt-in remote access over Tailscale: an embedded
  HTTP/SSE server, a browser editor, token authentication, Tailscale/loopback
  bind policy and a Settings › Remote panel (see the README and
  [`docs/remote.md`](docs/remote.md)).

## Reporting issues

- **Remote access, the web UI, `frankmd-remote`, or this fork's packaging** →
  open an issue in **this** repository.
- **Core editor, GPUI, Markdown, media, AI or anything present upstream** →
  report it to the
  [upstream repository](https://github.com/akitaonrails/FrankMDFast/issues).
  Do not file fork-specific issues upstream.
- **Security problems** → follow [`SECURITY.md`](SECURITY.md). Do not open a
  public issue for a vulnerability.

## Syncing with upstream

The local convention used here is:

```bash
git remote -v
# origin  git@github.com:akitaonrails/FrankMDFast.git   (upstream)
# fork    git@github.com:ghpo/FrankMDFast.git          (this fork)

git fetch origin
git switch master
git merge origin/master        # keep this fork's commits on top
```

If you prefer the opposite naming, use `upstream` for
`akitaonrails/FrankMDFast` and `origin` for your fork. What matters is that the
upstream remote is kept and its history is never rewritten.

Conflict-prone files are the ones this fork changes most: `README.md`,
`docs/architecture.md`, `assets/locales/native/*.yml`, the app's settings shell
and `Cargo.toml`/`Cargo.lock`. Resolve by keeping upstream changes first, then
re-applying the remote-access deltas.

## Contributing back

The upstream architecture notes remote/LAN mode as not planned, so do not send a
large pull request without asking first. If you want to propose a piece upstream,
open a focused issue/discussion with the upstream maintainers, keep the diff
small and respect their decision. This fork can carry the feature regardless.

## Packaging and releases

- Do **not** publish this fork under the upstream package names or AUR entries.
  Use a distinct name (for example `frankmdfast-remote`) and state that it is a
  fork.
- Include `LICENSE` and a link to this repository in any binary release.
- Prefer tags like `vX.Y.Z-remote.N` so a fork release is not confused with an
  upstream release.
- Credit the upstream project and link to it in release notes and announcements.
