<!--
  PR body for the DBX Store catalog pull request (t8y2/dbx-store).
  Copy everything below this comment into the PR description.

  用法：fork t8y2/dbx-store 后新建 PR，把下面正文粘进 PR 描述框。
  同时在本 PR 中加入两个文件：
    publishers/bobocha214.json                      <- store/publishers/bobocha214.json
    candidates/io.github.bobocha214.opcua.json      <- 由 node scripts/build-store-candidate.mjs 生成
  CI 在签名完成前会一直保持红色（open candidate(s) awaiting DBX Store signing），属正常。
-->

## Plugin submission

| | |
|---|---|
| Plugin ID | `io.github.bobocha214.opcua` |
| Version | `0.1.0` |
| Publisher | `bobocha214` |
| License | MIT (see `LICENSE`; matches `backend/Cargo.toml`) |
| Source | https://github.com/bobocha214/dbx-plugin-opcua/tree/v0.1.0 |
| Release | https://github.com/bobocha214/dbx-plugin-opcua/releases/tag/v0.1.0 |

### Capabilities

- `connection-provider` — adds an **OPC UA** connection type. Form fields: connection name, host, port (default `4840`), user name, password (stored as a secret; anonymous when left empty).
- `workbench` — **OPC UA Browser**, a three-column layout: the address space tree on the left (`Objects` / `Views` / `Types`), a drop board in the middle, the selected point's details on the right. Points are added by **dragging a `Variable` node** from the tree onto the board (non-`Variable` nodes are rejected with a hint); the board polls the selected points and a value can be edited in place to write it back immediately. Selecting a node or a board card shows its NodeId / BrowseName / DisplayName / NodeClass plus current value and write form.

Implemented sidecar methods: `plugin/initialize`, `connection/test`, `connection/connect`, `connection/disconnect`, `opcua/root`, `opcua/browse`, `opcua/read`, `opcua/readMany`, `opcua/write`.

Not implemented: method calls (`Call`), subscriptions/monitored items, historical reads, `Sign` / `SignAndEncrypt` security policies (only `None`).

### Permissions

- `host.workbench` — renders the plugin UI inside the DBX workbench.
- `host.connections:read` — reads the sanitized connection list so the UI can pick the OPC UA connection and pass its `connectionId` to the sidecar.

Not requested: `host.storage` (the watch list uses the page's own `localStorage`), `host.binary`, filesystem or clipboard permissions.

### Data access

- No filesystem, database, clipboard or `host.storage` access.
- Credentials never reach the UI: the host returns a **sanitized** connection summary (`id` / `providerId` / `name` / `label`) and only passes a `connectionId` to the sidecar. Host, port, user name and password are delivered to the sidecar by the host during `connection/connect` and are held in memory only for the lifetime of the session.
- No telemetry, no analytics, no data leaves the machine except the OPC UA traffic described below.

### Network access

- Single outbound TCP connection to the OPC UA endpoint entered by the user in the connection form (`host:port`, default `4840`). No other hosts are contacted, no fixed third-party endpoints.
- Security policy `None`, message security mode `None`; anonymous or user name/password identity token.
- The sidecar does not listen on any port and does not accept inbound connections.

### Native sidecar behavior

- Rust binary (`async-opcua 0.19`, pure Rust — no OpenSSL and no C toolchain dependency), built from this repository by the upstream release workflow.
- Communicates with the host over **stdio JSON-RPC 2.0** (`stdio-jsonl` v1). Logs go to **stderr**; stdout carries protocol frames only, written line-atomically so concurrent responses cannot interleave.
- Runs with the current OS user's privileges. It does not spawn child processes, does not write to the user's files, and does not modify the system.
- Connections are only opened when the host calls `connection/connect`; sessions are kept alive per connection id and closed on `connection/disconnect` or when the sidecar exits.
- Browse requests follow `HierarchicalReferences` forward and page through continuation points.

### Review aids

- Source: https://github.com/bobocha214/dbx-plugin-opcua/tree/v0.1.0
- Unsigned candidates and `release-candidates.json` are attached to release `v0.1.0`; the SHA-256 and size in `candidates/io.github.bobocha214.opcua.json` pin those exact bytes.
- `verified` is left to maintainers; this submission does not set it.
