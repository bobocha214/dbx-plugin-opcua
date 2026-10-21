## Plugin submission

- Plugin ID: `io.github.yuzhi.opcua`
- Version: `0.1.0`
- Publisher ID: `yuzhi`

## Source repository and tag

https://github.com/bobocha214/dbx-plugin-opcua

Built from the immutable tag `v0.1.0`. The candidates are the unsigned
`.dbxp` assets attached to that release.

## Capabilities and user workflow

The plugin contributes one connection provider (`OPC UA`) and its workbench:

- **Connection provider** — the user adds an OPC UA connection (host, port,
  optional user name and password), then tests it from the connection form.
  Failures surface the underlying reason returned by the OPC UA stack
  (unreachable endpoint, rejected identity, no matching endpoint).
- **Workbench** — an address space browser: expand nodes from the root folder,
  inspect attributes, and read or write the value of the selected node. Read and
  write are explicit user actions; the browser keeps no background subscription,
  no history read and no method call. Writes require a confirmation dialog that
  names the node id and the typed value.
- **No MCP contribution** — the plugin exposes no tool to the AI assistant.

## Permissions, data access, and network access

Manifest permissions: `host.workbench`, `host.connections:read`.

- `host.workbench` — renders the browser UI inside the DBX workbench sandbox.
- `host.connections:read` — reads the host, port and identity fields of the
  connection the user just opened so the sidecar can build the endpoint URL.
- **Network** — one outbound OPC UA TCP connection (`opc.tcp://`) to the host and
  port typed in the connection form (default 4840). Nothing else is contacted:
  no telemetry, no update check, no discovery service, no third-party API.
- **Credentials** — anonymous identity by default. If the user fills in a user
  name, the password is passed to the sidecar by the host secret binding and is
  never written to disk or logged.
- **Filesystem** — the plugin reads no user files and writes none. The client
  keypair is generated in memory per session and no certificate store is
  persisted.

## Native sidecar behavior

Yes — the plugin ships a Rust sidecar (`bin/<target>/dbx-plugin-opcua`), one
binary per platform, built from `backend/src`.

- Transport: `stdio-jsonl` over stdin/stdout, JSON-RPC 2.0. No listening
  sockets, no ports.
- Filesystem: no reads or writes; no data directory is used.
- Subprocesses: none. No external commands are executed.
- Network: only the outbound OPC UA session described above, opened on user
  action and closed on disconnect or when the workbench closes.
- Security policy: the session connects with the client's matching endpoint and
  trusts the server certificate (`trust_server_certs`), so there is no local
  trust store to manage.
- Dependencies: `async-opcua-client`, `async-opcua-types`, `serde`,
  `serde_json`, `tokio` — all pure Rust, no OpenSSL and no system TLS library.

## License

Apache-2.0 (see `LICENSE`).

## Homepage or support URL

https://github.com/bobocha214/dbx-plugin-opcua

## Confirmation

- [x] The candidate packages are unsigned and match the disclosed source tag.
- [x] I have not included private keys, tokens, credentials, or proprietary data.
- [x] The candidates are listed in `candidates/<plugin-id>.json`; `plugins/` and `catalog/` were not edited manually.
