## Plugin update

| | |
|---|---|
| Plugin ID | `io.github.yuzhi.opcua` (existing catalog entry, first published at `0.1.0` by merged PR #323) |
| Version | `0.2.1` |
| Publisher | `yuzhi` (unchanged — this is a version update, not a new listing or an identity transfer) |
| License | MIT (`LICENSE` inside the package); statically links MPL-2.0 dependencies — see `THIRD-PARTY` inside the package |
| Source | https://github.com/bobocha214/dbx-plugin-opcua/tree/v0.2.1 |
| Release | https://github.com/bobocha214/dbx-plugin-opcua/releases/tag/v0.2.1 |

`plugins/io.github.yuzhi.opcua.json` and `catalog/` were not edited by hand; the
only added file is `candidates/io.github.yuzhi.opcua.json`. `verified` is left to
maintainers.

## Response to the review on #370

| Point | What changed |
| --- | --- |
| Duplicate listing / identity transfer | The earlier submission in this PR used a new id `io.github.bobocha214.opcua` and a new publisher. That was wrong: it would have created a second entry and left 0.1.0 users without an upgrade path. This PR is updated in place and goes back to `io.github.yuzhi.opcua` + `yuzhi`, so 0.1.0 installs upgrade instead of getting a second entry. No separate new-id PR remains open. |
| `trust_server_certs(true)` hardcoded (undisclosed insecure default) | Removed. The default now verifies the server certificate. The connection form gained a boolean **"Trust any server certificate (insecure)"** (`trust_insecure`, default **off**); only when the user turns it on does the sidecar set `trust_server_certs(true)`, and it logs an explicit MITM warning to stderr first (`backend/src/main.rs`, `open_session`). |
| Client key pair written into the package | No longer true. `pki_dir()` points at `<system temp>/dbx-opcua-sidecar-pki`, outside the installed plugin directory, regenerated per run. The previous claim in the old PR body was wrong and has been corrected here. |
| No LICENSE / no third-party notice | `dbx-plugin.toml` now stages both: `[package] include = ["assets", "ui", "LICENSE", "THIRD-PARTY"]`. `THIRD-PARTY` carries the full MPL-2.0 text for the statically linked `async-opcua-client` / `async-opcua-types` 0.19.0 and their upstream source pointers; no MPL source files were modified. |
| Tag commit message did not match the pinned sha/size | `0.2.0` is abandoned. `0.2.1` is a fresh tag published from a clean rebuild: the sha256 and size in `candidates/io.github.yuzhi.opcua.json` are exactly the values carried by `release-candidates.json` on the release, which the release workflow writes from the bytes it uploaded. |
| Writes have no confirmation | Behaviour kept as-is (edit a watched value, `Enter` or moving focus writes it) but now stated instead of omitted: both the plugin `description` and its `zh-CN` localization say so — "Editing a watched value and pressing Enter (or moving focus) writes it immediately." No-op edits are dropped (`next !== current` short-circuits, so a blurred cell whose value did not change writes nothing), and writes read the value back to confirm it changed. |
| Description should name the connection target | Done — the description says it connects to the `opc.tcp://host:port` the user enters (default 4840) and that server certificates are verified by default. |

## Security notes

- **Default: certificate verification on.** No chain/hostname bypass unless the user opts in per connection.
- **Endpoint selection** still uses `connect_to_matching_endpoint`, so security policy and message mode are whatever the selected endpoint declares; only `SecurityPolicy#None` endpoints are usable today (same as 0.1.0 — `Sign`/`SignAndEncrypt` are not implemented). The username/password token therefore travels under the endpoint's policy; with the default settings that is what the server offers, and with the insecure toggle enabled the review's MITM scenario applies — which is exactly why the toggle warns about it.
- **Credentials** are delivered to the sidecar by the host during `connection/connect` and held in memory for the session lifetime only; they never reach the UI, which sees a sanitized connection summary (`id` / `providerId` / `name` / `label`).
- The only disk writes are the ephemeral client PKI material under the system temp directory.

## Capabilities

- **Connection provider** — connection name, host, port (default 4840), user name, password (bound to the host secret store; anonymous when empty), and the trust toggle described above. Capabilities: `test`, `connect`, `disconnect`.
- **Workbench ("OPC UA browser")** — three-column layout with draggable dividers: address space tree (`Objects` / `Views` / `Types`), attribute/details panel, and a watch board where `Variable` nodes are dropped. The board polls the watched points; a value can be edited in place. Writes resolve the node's `DataType` attribute server-side, surface the OPC UA status code on failure, and read the value back.
- Sidecar methods: `plugin/initialize`, `connection/test`, `connection/connect`, `connection/disconnect`, `opcua/root`, `opcua/browse`, `opcua/read`, `opcua/readMany`, `opcua/write`.
- Not implemented: method calls (`Call`), subscriptions / monitored items, historical reads, `Sign` / `SignAndEncrypt`.
- Icons: `assets/plugin.svg` is the official OPC UA wordmark SVG (512×512 canvas, the mark itself embedded as one base64 PNG — no external references, no `<script>`, no remote fonts); all three manifest icon slots (plugin, connection provider, workbench) point at it. `ui/icon.svg` is the same file with its `viewBox` cropped to the mark (`24 204 464 104`) so the workbench title bar can show it as a 96×22 lockup instead of a tiny square.

## Permissions, data access, and network access

Manifest permissions: `host.workbench`, `host.connections:read`.

- `host.workbench` — renders the plugin UI inside the host workbench sandbox.
- `host.connections:read` — reads the sanitized connection list so the UI can pass a `connectionId` to the sidecar.
- **Not requested:** `host.storage`, `host.binary`, filesystem or clipboard permissions.
- **Network** — one outbound `opc.tcp://host:port` to the endpoint the user typed (default 4840). Nothing else: no telemetry, no discovery service, no third-party API. The sidecar listens on no port and accepts no inbound connections.
- **Filesystem** — no user files read or written.

## Native sidecar behavior

Rust sidecar (`bin/<target>/dbx-plugin-opcua`), one binary per platform, built from `backend/src`.

- Transport: `stdio-jsonl` JSON-RPC 2.0 over stdin/stdout; logs go to stderr, written line-atomically.
- Subprocesses: none. Privileges: the current OS user's.
- Dependencies: `async-opcua-client` / `async-opcua-types` 0.19.0, `serde`, `serde_json`, `tokio` — pure Rust, no OpenSSL and no C toolchain.
- Sessions are opened only on `connection/connect` and closed on disconnect or when the sidecar exits.

## Build provenance

Candidates are the unsigned `.dbxp` assets attached to release `v0.2.1`, built by
`t8y2/dbx/.github/workflows/plugin-release-reusable.yml@main` on one native runner
per target — five targets: `linux-x64`, `linux-arm64`, `windows-x64`, `darwin-x64`,
`darwin-arm64`. SHA-256 and size are pinned to the bytes the release workflow
uploaded; `release-candidates.json` on the release carries the same values.

## License

MIT for the plugin code; the statically linked OPC UA crates are MPL-2.0 and a
`THIRD-PARTY` notice ships inside the package.

## Homepage or support URL

https://github.com/bobocha214/dbx-plugin-opcua

## Confirmation

- [x] The candidate packages are unsigned and match the disclosed source tag.
- [x] I have not included private keys, tokens, credentials, or proprietary data.
- [x] The candidates are listed in `candidates/io.github.yuzhi.opcua.json`;
      `plugins/` and `catalog/` were not edited manually.
- [x] `verified` is left to the maintainers.
