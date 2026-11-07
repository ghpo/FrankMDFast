# FrankMDFast architecture

Status: scaffold (2026-11-04). The stack is **GPUI + gpui-kit 0.9** (`docs/research/gpui-stack.md`).
Inputs to this design:
- `docs/research/components.md`: crate picks.
- `docs/research/history-pain-points.md`: the HP-0xx lessons, cited inline below.
- `docs/research/omarchy-4.5.md`: the OM-0xx requirements.
- `docs/inventory/{backend,frontend}.md`: the feature checklists and **[COMPAT]** quirks.

**Hard requirement: drop-in compatibility.** A notes folder must keep working, byte for byte,
when it moves between Rails FrankMD and FrankMDFast. That covers:
- the `.fed` format, its template, its write-back rules and its precedence,
- the two lock files,
- the note bytes, with no newline normalization,
- recents stored in `.fed`,
- the Hugo layout and slugs,
- theme and font ids.

The golden fixtures in `tests/fixtures/golden/` are the executable spec.

---

## 1. The GUI-free rule

> **All domain logic lives in crates that don't depend on GPUI.** `frankmd-app` is the only
> crate that links `gpui-kit`, and it only translates between snapshots and commands on one
> side and pixels and input on the other.

Enforcement:
- `cargo tree -p <crate> -i gpui-pre` must be empty for every crate except `frankmd-app`.
  A CI check runs it.
- `clippy.toml`'s `disallowed-methods` bans `std::fs::{write, rename, remove_*, create_dir*}`,
  `File::create*`, `OpenOptions::new` and `std::env::set_var` outside `frankmd-fs`. That
  crate opts out with `#![allow(clippy::disallowed_methods)]` (HP-004, HP-015).
- `disallowed-types` bans `reqwest::Client` outside `frankmd-net` (HP-010).
- `unsafe_code = "deny"` workspace-wide. `flock` and `renameat2` come through `rustix`.
- Every GUI-free crate is unit-testable with plain `#[test]`: no window, no runtime, injected
  clock and environment.

## 2. Crate layout

```
crates/
  frankmd-core       pure domain: DocId/Revision/Generation, RelPath, tree model+ordering, DocRegistry,
                     slugify (ACCENT_MAP), Hugo post gen, wikilinks/backlinks, recents, content-loss
                     guard, UTF-16⇄UTF-8 offsets, emoji data (embedded)          deps: serde, uuid, sha2, jiff
  frankmd-fs         THE filesystem writer: Vault, contain() (symlink-aware), atomic replace,
                     no-clobber create/rename, compat flock files, tolerant reads    deps: core, rustix
  frankmd-config     .fed line model + surgical write-back + template, schema (45 keys), casting,
                     ResolvedConfig (.fed > ENV > default, provenance, AI file-only mode), ConfigStore,
                     AppDirs (XDG)                                            deps: core, fs, theme, etcetera
  frankmd-i18n       Rails YAML (embedded unchanged), %{var}, Rails default plural, Locale + RTL flag
  frankmd-markdown   comrak → RenderDoc IR (allowlist by construction, sourcepos), LineMap, highlight
                     trait, math validation, YouTube embed parsing, front matter, HTML export
  frankmd-search     nucleo finder, ripgrep-lib content search, editor find/replace (JS semantics)
  frankmd-theme      Omarchy colors.toml cascade, builtin.json themes, custom theme, 38 UiTokens, contrast
  frankmd-watch      notify/debouncer → WatchEvent; self-write suppression; Omarchy/fontconfig/language watchers
  frankmd-vim        modal-editing state machine over a VimBuffer trait; ex-command → AppCommand
  frankmd-net        tokio runtime owner, TrustedClient, EgressClient (SSRF guard)      deps: tokio, reqwest
  frankmd-media      store_media() single upload pipeline, S3/local providers, library, web image search, YouTube
  frankmd-ai         AiProvider (generic OpenAI-compatible first), grammar fix, OpenRouter image gen
  frankmd-workspace  the authoritative store: Command → (Event, Effect); open docs, autosave, drafts
                     (DocId-keyed, on disk under the state dir), DocId index, startup, shell view
                     models (layout, focus, overlays, command registry, quick open, banners, status)
                                                   deps: core, fs, config, watch, search (+ markdown, i18n)
  frankmd-remote     optional remote web UI: axum HTTP/SSE, Tailscale bind policy, token auth,
                     embedded SPA                                          deps: core, fs, config, markdown,
                                                                           search, media, ai, net, watch
  frankmd-app        GPUI views + bridges (tokio, theme, vim, assets); binary `frankmd`
```

Changes from components.md's 11-crate proposal, and why:
- **`frankmd-fs` is split out of core.** That keeps core IO-free and makes "only one crate
  writes" enforceable by lint (HP-004).
- **`frankmd-workspace` is new.** HP-002 calls for a single store, and keeping it out of the
  GUI crate makes the riskiest logic testable: autosave, rename races, drafts.
- **`frankmd-vim` is new.** GPUI has no vim, and the engine is large enough to deserve its
  own test suite (gpui-stack.md §6).
- The GUI crate is `frankmd-app`, using GPUI instead of GTK. katex-rs and webkit6 are gone.

Dependency direction (acyclic):

```
app → workspace → {config → {fs → core, theme}, watch → core}
workspace → {search, markdown, i18n} → core      (view models; vim stays out: VMs take a ModeLabel)
app → {markdown, search, theme, i18n, vim} → core (markdown, search only)
app → {media, ai} → net ;  media → {config, fs}
```

`workspace → {markdown, search, i18n}` is allowed (ui-test-plan D-1) so view models can call
`markdown::{table, edit, linemap}`, `search::{finder, find}` and resolve labels; today only
`search` is used. `config → theme` resolves the theme settings. The graph stays acyclic: none
of these depend on workspace.

## 3. Domain design decisions (keyed to the history lessons)

### 3.1 Identity: `DocId`, not paths (HP-001)

- `DocId` is a UUIDv7. `frankmd_core::registry::DocRegistry` is the **only** map
  `DocId ⇄ RelPath`.
- Autosave timers, drafts, the undo-state cache (LRU of 20), recents projection, tree
  expansion and the preview cache are all keyed by `DocId`.
- A rename or move (file or folder) is one call, `DocRegistry::rename(from, to)`. It returns
  `Vec<Remap>`, and that list becomes the single `Event::Remapped`. Subscribers update from
  it and nothing re-implements prefix matching. `RelPath::starts_with` is segment-wise and
  never a string prefix (HP-008).
- Identity persists across restarts in `$XDG_STATE_HOME/frankmd/vaults/<hash>/doc_index.json`
  (`frankmd_workspace::doc_index`, path → DocId). Until `DocRegistry` can be seeded with known
  ids, the store re-keys a note's drafts from its persisted id to the id minted this run when it
  opens the note. It **never** goes into the vault or into front matter
  (compat). External renames detected by the watcher are matched by `(revision, basename)`
  when the old path vanishes in the same debounced batch. Otherwise they get a new DocId.
- Saves name `(DocId, edit Generation)`. A save whose doc was deleted or renamed in the
  meantime resolves the *current* path through the registry. If the doc is gone, the save
  becomes a draft and is never a create (HP-006, bug #170).

### 3.2 One source of truth (HP-002, HP-014)

- `frankmd-workspace::Workspace` owns the vault, `ConfigStore`, `DocRegistry`, open docs,
  the tree and its generation.
- The GPUI layer wraps it in one `Entity<WorkspaceModel>`. Views read `Snapshot`s (with
  `Arc`-shared tree and config) and send `Command`s.
- The editor's rope is the **only** live copy of unsaved text. The workspace receives
  `Edited{doc, text}` notifications (or incremental edits) to drive autosave and drafts,
  and it never keeps a competing buffer.
- Policy is decided in one place:
  - The storage provider (local/S3) is resolved by `ResolvedConfig`.
  - The Hugo default folder is resolved by the workspace and shown by the New Note dialog.
  - The upload destination is resolved by `store_media`.

  The UI displays the result and never decides it.

### 3.3 Filesystem mutations (HP-004, HP-005, HP-006, HP-028)

`frankmd_fs::Vault` is the only path to disk:

| Operation | Semantics |
|---|---|
| `contain(&RelPath)` | Lexical normalization plus a per-component `lstat` walk. Rejects escaping symlinks, NUL, absolute paths and reserved app files. Returns `ContainedPath`, the only type the write APIs accept (HP-008). |
| `update_existing` | Atomic replace (same-dir temp `.<name>.<12hex>.tmp` (**[COMPAT]**), fsync, keep mode, `rename`). `NotFound` if the target is gone, and **it never creates** (HP-006). |
| `create_note` / `create_folder` | No-clobber: `O_EXCL` temp + `link(2)`, falling back to `renameat2(RENAME_NOREPLACE)`. `EEXIST` maps to `AlreadyExists`. No `exists()` checks (HP-005). |
| `rename` | `RENAME_NOREPLACE`. EXDEV falls back to copy and delete. Folder renames also update the Hugo `index.md` slug (**[COMPAT]**). |
| `delete_note(expected_revision)` | Returns `RevisionConflict` on a revision mismatch (**[COMPAT]** 409 semantics). |
| `delete_empty_folder` | Hidden files count, so a stray file blocks the delete (**[COMPAT]**). |
| Locks | `flock` on `.frankmd-config.lock` (around `.fed` read-modify-write and recents) and `.frankmd-filesystem.lock` (EX for mutations, SH for tree reads), in the notes root, with the same names and the same `O_NOFOLLOW` 0600 open. A Rails instance sharing the folder coordinates with us. Fallback lock dir: `$XDG_STATE_HOME/frankmd/locks/<sha256(realpath)>/` (instead of Rails' `tmp/`). |
| Reads | Exact bytes and `Revision = sha256`. Non-UTF-8 files open read-only with a banner instead of failing. The search and tree walkers skip and report vanished or unreadable files (HP-028). |

Multi-item operations (bulk move/delete) de-duplicate ancestors, stop at the first failure,
report partial results, and emit `Reveal` for the destination before the tree re-renders
(HP-029).

### 3.4 App-owned data outside the vault (HP-012, OM-010)

| Data | Location |
|---|---|
| Per-folder config **[COMPAT]** | `<notes>/.fed`: same format, mode 0600 after any write |
| Lock files **[COMPAT]** | `<notes>/.frankmd-config.lock`, `<notes>/.frankmd-filesystem.lock` |
| Recent files **[COMPAT]** | inside `.fed` (`recent_files` JSON), so they sync with the vault |
| Hugo template (user-owned) | `<notes>/.hugo_template.md` (read only) |
| Media (user content) | `<notes>/images/`, `<notes>/videos/` (**[COMPAT]** paths, hidden at the root unless they contain notes) |
| Global settings, user themes | `$XDG_CONFIG_HOME/frankmd/` |
| DocId index, drafts, undo snapshots, window/session state, logs, fallback locks | `$XDG_STATE_HOME/frankmd/` (per-vault subdir keyed by the realpath hash) |
| Thumbnails, math raster cache, remote image cache | `$XDG_CACHE_HOME/frankmd/` |

The tree walker, search, the asset resolver and the watcher exclude dotfiles by
construction, so `.fed` and the locks never show up (the **[COMPAT]** rule). No other app
file is ever created in the vault. In-app editing of `.fed` stays disabled, as upstream did
after `b03f36c`. A Settings "Open .fed in external editor" action can replace it.

### 3.5 Single upload pipeline (HP-015, §3.9 of the history doc)

Drop, paste, the picker, URL import and AI generation all call `frankmd_media::store_media`.
It runs these steps in order:

1. The extension allowlist from `image_upload_extensions`/`video_upload_extensions`
   (**[COMPAT]** parsing).
2. A magic-byte check (`infer`), so the content must match the extension.
3. A size cap.
4. Optional resize/re-encode (`image` + `fast_image_resize`; ImageMagick is no longer
   needed).
5. A random collision-free name and a no-clobber write (local) or a no-overwrite
   `PutObject` with `If-None-Match: *` (S3).
6. The provider resolved from config, with recovery to a visible warning when S3 is
   selected but unusable.

Remote URLs come in only through `EgressClient`.

### 3.6 Egress policy in one client (HP-010)

`frankmd_net`:

- **`TrustedClient`** (AI providers, S3 endpoint, YouTube, Google CSE): https/http only,
  `.no_proxy()`, timeouts, body caps.
- **`EgressClient`** (any user-supplied URL):
  - `check_url` (scheme, no userinfo),
  - a `GuardedResolver` that drops every non-public IP (**[COMPAT]** `egress_policy.rb`
    table, golden `egress_policy.json`), so the connection is pinned to vetted addresses
    and DNS rebinding is impossible,
  - per-hop redirect re-validation and a hop limit,
  - a 25 MB streamed cap,
  - no proxy.
- Error bodies from upstream are never shown or logged (HP-011). Secrets live in
  `secrecy::SecretString` with a redacting `Debug`.

### 3.7 Explicit startup phases (HP-016)

`frankmd_workspace::startup::Phase` runs in this order:

1. `Boot`: CLI, logging, XDG.
2. `ConfigLoaded`: vault opened, `.fed` ensured/upgraded, locale and theme resolved.
3. `IndexReady`: first tree scan, DocId index, watchers armed.
4. `UiMounted`.
5. `DocumentOpened`: deep link or last note, draft recovery.

Nothing reads state from a later phase. The window opens at `UiMounted` with the tree
already present, so there's no splash race (#180).

### 3.8 Layout modes as pure functions (HP-019)

Scroll sync (lock source, thresholds, ratio snapping), typewriter rows, and editor width
are pure functions in `frankmd-workspace` (inputs: the line map, viewport metrics, cursor
line). Views call them from resize, visibility and scroll handlers and apply the result
only to their own pane. They're tested against `js_scroll_utils.json`,
`js_markdown_line_mapper.json` and `js_typewriter_utils.json` style goldens.

### 3.9 One command and keybinding registry (HP-018)

Every user action is a `Command` (or a view-local action) with one GPUI `Action` type.
Key bindings are registered in one table in `frankmd-app` with explicit precedence:

1. The vim capture layer, when vim mode is on.
2. Editor bindings (`Input && mode == editor` context).
3. Workspace globals.

Vim ex-commands and the CLI dispatch the same commands. A test asserts there are no
duplicate chords per context and that the **[COMPAT]** non-macOS passthrough list
(`Ctrl-F/N/P/E/B/I/V`) reaches the app in vim normal mode.

## 4. Configuration resolution model (exactly the original's)

Implemented in `frankmd_config::ResolvedConfig`. **This is not "improved".** The goldens
`config_parse.json`, `config_env.json`, `config_writeback.json` and `config_template.json`
lock it down. Summary of backend.md §1.3–1.8:

1. **`get(key)`**:
   - If the key is **present** in `.fed`, use its value, even when it's empty. An empty
     value means `None` and blocks ENV.
   - Otherwise, if the schema has an `env` name and the ENV value is `present?`
     (non-blank after trim), cast it.
   - Otherwise use the schema default.
2. **`get_ai(key)`**: if any of the 5 AI credential keys is present in `.fed` (even
   empty), ENV is ignored for **all** AI keys (credentials, `ai_provider`, `ai_model`,
   `*_model`). Provider availability order and the `effective_ai_provider` priority are
   copied verbatim.
3. **Image generation exception**: `.fed openrouter_api_key` (if non-nil), otherwise
   `ENV OPENROUTER_API_KEY`. This bypasses file-only mode.
4. **Casting**: Ruby `to_i` for integers (`"12px"` → 12, `"abc"` → 0). Booleans are true
   only for `true|1|yes|on`. Blank means nil.
5. **Parsing**: there are no inline comments, the first `=` splits, `"…"` and then `'…'`
   are stripped, keys are lowercased, the last duplicate wins, and a bad JSON line is
   skipped with a warning.
6. **Write-back**:
   - The first matching `# key =` comment is uncommented in place.
   - All active duplicates are replaced.
   - Missing keys are appended at the end.
   - The output is LF with a single trailing newline.
   - Strings containing a space or `=` are double-quoted. Quotes, CR and LF are rejected
     (no line injection).
   - Writes are all-or-nothing, atomic, mode 0600, under the config lock, and re-read the
     file first.
7. **File creation and upgrade**: on open, a missing `.fed` is created from the verbatim
   3032-byte template, and an old one gets the AI section appended under the original's
   exact condition. A read-only vault degrades silently (**[COMPAT]**). One consequence:
   opening a folder creates `.fed` and the lock files, exactly as before.
8. **ENV**: `EnvSnapshot::capture()` runs once at boot, after optional `.env` loading with
   `dotenvy`. That snapshot is the only ENV access. `NOTES_PATH` (not a `.fed` key), the
   `IMAGES_PATH` boot default (`XDG_PICTURES_DIR` or `~/Pictures` when unset), and
   `FRANKMD_LOCALE` keep their roles.

Locale: the original's precedence is `params[:locale]` > `.fed locale` > `FRANKMD_LOCALE` >
`en`. We keep it exactly, with `--locale` playing the role of `params[:locale]`.

omarchy-4.5.md §4.3 proposes moving ENV above `.fed` and following the Omarchy language.
**That is rejected for the default path**, because compatibility wins. Following the
system language is an **opt-in** global setting (`$XDG_CONFIG_HOME/frankmd/config.toml`
`locale = "system"`). It only stands in for the final `en` default when `.fed` has no
`locale` key and `FRANKMD_LOCALE` is unset.

The same rule applies to theme: `.fed theme` > default. The Omarchy follow mode is the id
`omarchy` (**[COMPAT]**). A missing theme on a detected Omarchy system defaults to
`omarchy`, which is the only default divergence. We note it in the release notes.

Provenance: every `Resolved` carries `Fed | Env(name) | Default`, and Settings displays it
(for example "set by ENV AWS_REGION; .fed overrides"). That addresses the #214 confusion
without changing the semantics. Load warnings (skipped lines, invalid provider) appear in
the UI instead of failing silently.

Live reload: `.fed` is watched. External edits, such as a sync tool or a Rails instance,
rebuild `ResolvedConfig` and emit `SettingsChanged`. Read-modify-write always re-reads
under the lock.

## 5. Threading and async model

| Work | Where |
|---|---|
| UI, entity updates, layout, paint | GPUI foreground (main thread) |
| Markdown render → IR, code highlighting, math raster, fuzzy ranking, content search, diffs, tree scans, file reads/writes | GPUI `background_executor()` (a thread pool). The result is applied with `cx.spawn` → `entity.update` and tagged with a `Generation`; stale results are dropped (HP-003). |
| HTTP: AI, S3, YouTube, Google CSE, DuckDuckGo, URL import | **One tokio multi-thread runtime (2 workers)** owned by `frankmd_net::NetRuntime`, stored as a GPUI `Global`. `bridge::tokio::spawn_net` spawns on tokio and awaits the `JoinHandle` from a GPUI background task. GPUI pulls in no tokio. |
| File watching | `notify`'s own thread → debouncer → `async-channel` → a GPUI foreground task, which turns each event into `Command::External(WatchEvent)` |
| Timers (autosave ticks, debounces) | GPUI `background_executor().timer()` drives `Command::Tick`. The decision logic uses the injected `Clock`, so tests are deterministic. |

Rules:
- No blocking IO on the foreground thread.
- Every long operation is cancellable: dropping the GPUI `Task` cancels it, and tokio tasks
  are aborted through their `JoinHandle`.
- Every operation is budgeted: highlight N blocks / M bytes, a max number of search
  results, a decode cap, LRU caches (HP-017).
- Workspace commands are applied **serially** on the foreground thread. Filesystem effects
  that take the lock run in the background but are queued per vault, one at a time. That
  gives one serialized mutation queue (HP-005 actor).

## 6. Event flow

```
 keystroke ─► [vim capture?] ─► Editor (rope) ──InputEditsEvent──► EditorPane
                                                     │ Command::Edited{doc,text}
                                                     ▼
             ┌────────────── WorkspaceModel (Entity wrapping frankmd_workspace::Workspace) ─────────────┐
 Commands ──►│ handle(cmd) → (Vec<Event>, Vec<Effect>)                                                  │
 (menus,     │   registry / config / docs / tree                                                        │
  shortcuts, └───────┬──────────────────────────────────────────────┬───────────────────────────────────┘
  ex, CLI,           │ cx.emit(Event) ─► views re-render from Snapshot  │ Effects ─► background executor / tokio
  watcher)           ▼                                               ▼             │
               Explorer, Editor, Preview, Dialogs                 results ─► Command::*Completed{generation}
```

Preview flow: `Edited` → a 150 ms debounce → `Effect::Render{doc, generation}` → background
`frankmd_markdown::render` → `Command::Rendered{generation, Arc<RenderDoc>}` → the preview
view swaps its `ListState` items while keeping its `(block, offset)` anchor.

## 7. File watching

- **Vault**: recursive `notify` with `notify-debouncer-full` at 200 ms. Dotfiles are ignored
  except `.fed`. `SelfWrites` tokens (path + revision registered before each write)
  suppress echoes.
  - An external change to an open **clean** doc reloads it and keeps the caret.
  - An external change to a **dirty** doc raises `DocConflict`, and a diff dialog lets the
    user choose.
  - Tree changes trigger a rescan, tagged with a generation.
- **`.fed`**: rebuild the config (see §4).
- **Omarchy theme**: watch the `~/.local/state/omarchy/current/` directory (`theme/` is
  replaced on every switch). Debounce 100 ms, re-arm, and fall back to the nearest
  existing ancestor. Re-read `colors.toml`, keep the last good palette when a read races
  the swap (HP-024), and compare the palette content, not the name (OM-003).
- **`~/.config/omarchy/language`** (only when the opt-in system locale is on) and
  **`~/.config/fontconfig/`**: re-resolve.

## 8. Error handling

- Each library crate has a `thiserror` enum (`FsError`, `ConfigError`, `NetError`,
  `MediaError`, `AiError`, …). `anyhow` is allowed only in `frankmd-app` glue and the CLI.
- At the workspace boundary, errors become `UserError { key: i18n key, args }`. They are
  localized and never carry raw OS or upstream text (HP-011). They're shown as toasts or
  inline dialogs.
- The **[COMPAT]** user-visible messages (`errors.note_already_exists`,
  `errors.parent_folder_not_found`, …) reuse the Rails locale keys.
- Data-safety paths never lose text. A failed save keeps a draft, a failed draft write is
  shown to the user, and the app's close handler flushes dirty docs and drafts before it
  quits (HP-007).
- Panics: the app installs a panic hook that flushes drafts synchronously and logs the
  panic. Pure crates don't catch panics.

## 9. Logging

- `tracing` everywhere. GPUI and gpui-kit log through `log`, which we bridge with
  `tracing-log`.
- Subscriber: `FRANKMD_LOG` (EnvFilter syntax, default `info`) → stderr, plus a rolling
  file in `$XDG_STATE_HOME/frankmd/logs/` (`tracing-appender`, daily, keep 7).
- Secrets never reach logs: `SecretString`, redacted `Debug`, and the config log view masks
  `secret` schema keys (**[COMPAT]** `SENSITIVE_KEYS`).
- Spans: `vault.op`, `render`, `search`, `net.request` (host only, no query string).

## 10. Testing strategy (HP-025, HP-026, HP-027)

1. **Conformance tests** in each GUI-free crate, against `tests/fixtures/golden/*.json`
   (Ruby and JS outputs of the original). These are the next step after this scaffold.
2. Property tests (proptest): `.fed` round-trip, slug idempotence, the egress IP
   classifier, `RelPath` normalization, and the vim engine's invariants.
3. Filesystem integration tests: a fresh `tempfile` dir per test, assertions on specific
   artifacts, the regression matrix from history §7 first (HP-027).
4. Workspace tests with a fake clock: rename during a pending autosave, delete during a
   save, drafts through a parent-folder move, stale tree results.
5. GPUI UI tests (`#[gpui_kit::test]`, `gpui_kit::test`): keymap precedence, vim
   capture-before-editor, find/replace wiring, typewriter centering, plus a few WGPU
   screenshot checks on lavapipe in CI.
6. CI runs every tier with a clean HOME/XDG and pinned runner images.

## 11. Assets reused from FrankMD (`assets/`, generated by `tools/assets/extract.mjs`)

| Asset | Consumer | How |
|---|---|---|
| `locales/*.yml` | `frankmd-i18n` | `include_str!`, loaded **unchanged** (`Locale::source_yaml`) |
| `themes/builtin.json` (18 themes × 38 roles) | `frankmd-theme` | `include_str!` (`BUILTIN_THEMES_JSON`) → `UiTokens` → gpui-kit `ThemeConfig` |
| `emoji/shortcodes.json`, `emoji/emoticons.json`, `emoji/names/*.json` | `frankmd-core::emoji` → pickers, `:shortcode:` in the preview | `include_str!` |
| `icons/phosphor.json` (201 Phosphor icons, MIT) | `frankmd-app::assets::FrankAssets` | Synthesizes `icons/ph/<name>.svg`. It backs the icon picker, and through `set_icon_resolver` it replaces Lucide for chrome icons that have a Phosphor equivalent. Lucide stays the fallback (gpui-stack.md §2.2). |
| `brand/icon.svg` and PNGs | packaging (`.desktop`/hicolor, OM-013) and the about/empty states | `include_bytes!` |
| `fonts/web/*.woff2` | none (reference) | `editor_font` ids are resolved to installed families through fontconfig |
| `fonts/katex/*.woff2` | none | RaTeX embeds its own TTF KaTeX fonts |

`assets/` and `tools/assets/` are generated, so never hand-edit them. Crates reference them
with `../../../assets/...` relative paths.

## 12. Open decisions

- Exact Phosphor ↔ Lucide alias table. To be written with the icon work.
- Whether to vendor `gpui-bidi` for preview RTL (gpui-stack.md §7).
- Whether `gpui-fast` replaces the upstream snapshot (spike S8).
- Remote/LAN mode: implemented as an opt-in crate (`frankmd-remote`): Tailscale
  bind by default, mandatory >= 32-byte token, loopback as an alternative, no
  wildcard bind without an explicit flag (HP-023). See `docs/remote.md`.
