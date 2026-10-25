# DBX OPC UA 插件

参考 [DBX 插件开发文档](https://dbxio.com/cn/docs/plugin-development) 实现的 **OPC UA 客户端读写插件**：连接 OPC UA 服务器 / PLC、浏览地址空间、读取与写入节点值。

- 后端：Rust + [`async-opcua`](https://crates.io/crates/async-opcua)（纯 Rust，无需 OpenSSL），自行实现 DBX Sidecar Protocol v1（文档允许用任意语言实现该协议）
- 前端：沙箱工作台（原生 HTML/CSS/JS，classic script），仅通过 `window.dbxPlugin` Host Bridge 通信
- 传输：`stdio-jsonl`，JSON-RPC 2.0（stdout 只输出协议消息，日志走 stderr）

> 功能范围：**连接 + 地址空间浏览 + 读 + 写**。未包含方法调用（Call）、订阅监控（Subscription）、历史读数（HistoryRead）。安全策略仅 `None`（匿名或用户名/密码）。

## 目录结构

```
dbx-plugin-opcua/
├── manifest.json              # 插件契约：身份、权限、入口、contributions
├── dbx-plugin.toml            # 包描述符（schema_version / backend / package.include）
├── assets/                    # plugin.svg、connection.svg
├── ui/                        # 沙箱工作台（index.html、styles.css、app.js、icon.svg）
├── backend/                   # Rust sidecar（Cargo.toml、src/main.rs）
├── scripts/                   # build-store-candidate.mjs（生成商店候选元数据）
├── store/                     # 上架 DBX Store 用的材料（publisher 记录、候选元数据、PR 正文）
├── .github/workflows/         # plugin-release.yml（上游 reusable workflow，产出多平台包）
├── LICENSE                    # MIT
└── dist/                      # 打包输出（不提交 Git）
```

## 打包

使用官方 CLI（Node 包，内含预编译二进制）：

```bash
npm install --global @dbx-app/plugin-cli
dbx-plugin package .                     # 为当前平台构建并打包
dbx-plugin package . --target windows-x64 --output-dir dist
```

产物（未签名审核候选包）：

```
dist/io.github.bobocha214.opcua-0.1.0-<target>.dbxp
dist/io.github.bobocha214.opcua-0.1.0-<target>.artifact.json
```

本地安装测试：在 DBX 插件中心开启「允许安装未签名开发包」后安装该 `.dbxp`。

> CLI 会**自动为当前主机平台构建 Rust sidecar** 并放入包内 `bin/`，因此不需要手工 `cargo build` 再拷贝。
>
> 打包时 CLI 还会**自动改写包内 manifest 的 `entrypoints.backend.executable`**，指向该平台的实际路径（例如 Windows 包内为 `bin/windows-x64/dbx-plugin-opcua.exe`）。因此源码里的 `manifest.json` 保持通用路径 `bin/dbx-plugin-opcua` 即可，无需为每个平台维护一份 manifest。

## 多平台支持

官方文档要求：**包含原生 Sidecar 的插件需按操作系统和 CPU 架构分别构建**，不存在一个跨平台的通用包（只有纯前端插件才能打 `universal` 包）。

因此多平台通过 CI 实现：`.github/workflows/plugin-release.yml` 复用上游的 `t8y2/dbx/.github/workflows/plugin-release-reusable.yml`（pin 到 commit SHA，不用 `@plugin-sdk-v1` —— 该 ref 在上游并不存在），在 `release: published` 时**按目标平台各构建一个 `.dbxp`**，随 Release 发布：

| 平台 | target |
|------|--------|
| Windows x64 | `windows-x64` |
| Linux x64 | `linux-x64` |
| Linux ARM64 | `linux-arm64` |
| macOS Intel | `darwin-x64` |
| macOS Apple Silicon | `darwin-arm64` |

（上表即上游 reusable workflow 的默认 `build-matrix`；本仓库未覆盖它。）

Release 需包含：每个平台的 `.dbxp` + 对应 `.artifact.json` + 汇总的 `release-candidates.json` + Release Notes 与源码 Tag。**不要把 `.dbxp` 提交进 Git 历史。**

在本机只打当前平台包（例如 Windows）：

```bash
dbx-plugin package . --target windows-x64
```

> 离线环境：若无法访问 crates.io，可加 `CARGO_NET_OFFLINE=true`（依赖需已缓存）。

## 上架官方商店（DBX Store）

官方目录仓库：[t8y2/dbx-store](https://github.com/t8y2/dbx-store)（规范见其 `README.zh-CN.md` 与 `CONTRIBUTING.md`）。插件源码与未签名候选 Release **留在本仓库**，商店只收目录元数据，`.dbxp` 二进制不能提交进 Git。

### 1. 发布未签名候选包

```bash
git tag v0.1.0 && git push origin v0.1.0
```

在 GitHub 上发布该 Tag 的 Release，`plugin-release.yml` 会自动为 5 个平台各构建一个 `.dbxp`，并把包与汇总的 `release-candidates.json` 上传到该 Release。

### 2. 生成候选元数据

```bash
# 仅本地已构建的平台（默认读取 dist/*.artifact.json）
node scripts/build-store-candidate.mjs --repo bobocha214/dbx-plugin-opcua --tag v0.1.0

# Release 已发布、拿到全部平台后（下载 Release 里的 release-candidates.json）
node scripts/build-store-candidate.mjs --repo bobocha214/dbx-plugin-opcua --tag v0.1.0 \
  --release-candidates ./release-candidates.json
```

脚本从**真实构建产物**读取 SHA-256 与大小写入 `store/candidates/io.github.bobocha214.opcua.json`，不要手抄这两个值——签名工作流会逐字节比对，不一致直接拒绝。重新打包后哈希会变，必须重新生成。

> 汇总文件不会自动识别：只有显式传 `--release-candidates` 才会使用。因为一份与实际上传字节不符的陈旧清单会 pin 错哈希。

商店展示文案（name / description / tags / license / localizations / releaseNotes）取自 `.dbx-store.json`——这也是商店同步器读取的同一份元数据。要改文案就改那里再重跑脚本，不要直接手改 `store/candidates/*.json`（下次生成会被覆盖）。

> `store/make-candidate.mjs` 是功能等价的早期脚本，但它**必须**传 `--release-candidates`（只适用于 Release 已发布之后）。本地打包完就想生成候选时用 `scripts/` 下这个。

### 3. 提交 catalog PR

fork `t8y2/dbx-store`，在 PR 中加入两个文件（**只加这两个**）：

| 本仓库文件 | 放到 dbx-store 的 |
|---|---|
| `store/publishers/bobocha214.json` | `publishers/bobocha214.json` |
| `store/candidates/io.github.bobocha214.opcua.json` | `candidates/io.github.bobocha214.opcua.json` |

PR 正文用 `store/PULL_REQUEST.md`（含 capabilities、permissions、data access、network access、native sidecar behavior）。

- CI 在签名完成前**保持红色**（`open candidate(s) awaiting DBX Store signing`），这是设计如此，不要去修。
- 维护者会在 PR 下评论 `/sign` 触发受保护签名工作流，它把最终 `plugins/<id>.json` 与 `catalog/index.json` 写回同一个 PR，然后由维护者合并。
- 不得提交：`.dbxp` 二进制、插件源码、私钥/Token；不得自填 `verified: true`；候选 URL 不能引用 `dbx-store` 自己的 Release。

### 当前状态

本机只能构建 `windows-x64`（无 Linux/macOS linker，也无法交叉编译），因此仓库里的候选元数据目前**只含 windows-x64**。其余 4 个平台请在 Release 构建完成后，用 `--release-candidates` 重新生成，覆盖 `targets` 后再提交/更新 PR。

## 使用

### 建立连接
连接表单由 `connection-provider` contribution 声明，DBX 负责渲染、保存与生命周期管理：

| 字段 | binding | 说明 |
|------|---------|------|
| 连接名称 | `name` | 显示名 |
| 主机 | `host` | 如 `192.168.0.10` |
| 端口 | `port` | 默认 `4840` |
| 用户名 | `username` | 可选，留空为匿名 |
| 密码 | `secret` | 可选，按敏感字段存储 |

> 安全边界：宿主返回给 UI 的连接列表是**脱敏的**（形如 `{ id, providerId, name, label }`，不含主机、端口、凭据）。因此 UI 不自行发起连接，只使用宿主已建立连接的 `connectionId`。

### 浏览 / 读 / 写
- 左侧树加载三个标准根：`Objects` (`i=85`)、`Types` (`i=86`)、`Views` (`i=87`)；按 `HierarchicalReferences` 前向展开，自动翻页 continuation point。
- 选中 `Variable` 节点可读取 `Value`（含数据类型、状态码、时间戳），或选择数据类型写入标量值。

### 监视点（右侧持续观察 + 就地改写下发）
- 添加：树上 `Variable` 节点行悬停出现「＋」，或详情面板点「加入监视列表」。只有 `Variable` 可加入。
- 观察：点「▶ 持续观察」，按 200ms / 500ms / 1s / 2s / 5s 间隔轮询；走 **`opcua/readMany` 一次批量读**，N 个点只花一次往返。上一轮未返回时自动跳过本轮，避免堆积。
- 改值下发：**鼠标点击数值** → 变成已全选的输入框 → 改完回车（或失焦）即 `opcua/write` 下发；`Esc` 取消。写入成功后自动回读该点，单元格绿色闪烁，失败红色闪烁并提示。
- 轮询期间**不会覆盖你正在编辑的单元格**；写入用的数据类型取该点最近一次读到的 `dataType`。
- 监视点按连接 id 存 `localStorage`，切换连接后自动恢复；`×` 移除，「清空」全清。
- 轮询失败会停止观察并提示（常见原因是会话未建立，需先在 DBX 中打开该连接以触发 `connection/connect`）。

## Sidecar 协议

`plugin/initialize` 握手响应必须包含 `protocolVersion`、`capabilities` 与 `plugin { id, version }`，且 id / version 必须与 `manifest.json` 一致，否则宿主报 `Sidecar identity or protocol does not match manifest`。

| 方法 | 参数 | 说明 |
|------|------|------|
| `plugin/initialize` | `{ host: { protocolVersions } }` | 协议协商与身份上报 |
| `connection/test` | `{ connection }` | 探测端点后关闭 |
| `connection/connect` | `{ connection }` | 建立会话，按连接 id 保活 |
| `connection/disconnect` | `{ id }` | 释放会话 |
| `opcua/root` | `{ id }` / `{ connectionId }` | 三个标准根 |
| `opcua/browse` | `{ id, nodeId }` | 子节点列表 |
| `opcua/read` | `{ id, nodeId }` | 读 `Value` |
| `opcua/readMany` | `{ id, nodeIds: [...] }` | 一次读多个 `Value`（监视点轮询用，上限 500） |
| `opcua/write` | `{ id, nodeId, dataType, value }` | 写标量值 |

连接 id 同时接受 `id`、`connectionId` 与嵌套 `connection.id`。`nodeIds` 同时接受 `["i=85"]` 与 `[{"nodeId":"i=85"}]`。

## 待替换 / 已知限制

- 插件身份：`id` = `io.github.bobocha214.opcua`，`publisher` = `bobocha214`，与 GitHub 账号 `bobocha214` 及仓库 `https://github.com/bobocha214/dbx-plugin-opcua` 一致；`source` / `homepage` 指向该仓库。改动其中任何一项都会改变包内容，需重新打包并重新生成候选元数据。
- 许可证：仓库 `LICENSE` 与 `backend/Cargo.toml` 均为 **MIT**，商店条目的 `license` 字段保持一致。
- 仅 `None` 安全策略；写入仅支持标量类型；未实现订阅与方法调用。
- 本插件自行实现 JSON-RPC（未使用官方 `dbx-plugin-sdk`），因为 `async-opcua` 需要常驻 tokio runtime 驱动会话事件循环，而 SDK 的 handler 是同步的。协议行为已按 SDK 源码对齐。

## 排错：`invalid type: map, expected a sequence`

宿主报 `Plugin '<id>' initialization failed: Failed to decode plugin '<id>' result: invalid type: map, expected a sequence`，原因是 `plugin/initialize` 响应的 **`capabilities` 必须是字符串数组**，不能是对象。

宿主侧按 `Vec<String>` 反序列化该字段（官方 Rust SDK 的 `PluginMetadata.capabilities: Vec<String>`，Go SDK 的 `Capabilities []string`，构造方式为 `.with_capability("connections")`）。正确响应：

```json
{
  "protocolVersion": 1,
  "capabilities": ["connections"],
  "plugin": { "id": "io.github.bobocha214.opcua", "version": "0.1.0" }
}
```

同理，`connection/test` 应返回 `{ "success": true, "message": "..." }`，`connection/connect` / `connection/disconnect` 应返回 `{ "success": true }`。

## 排错：连接下拉框里出现其他协议的连接（如 Modbus device）

`getConnections()` 返回的是**所有插件**的脱敏连接，且摘要字段只有 `{ id, providerId, name, label }` —— **没有 `dbType` / `dialect`**。

如果按 `dbType === "opcua"` 过滤，条件永远不成立，代码就会 fallback 到列表第一项，把别的插件的连接（例如 Modbus device）当成当前连接。本插件的处理方式：

1. 用 `providerId` / `pluginId` / `connectionType` / `databaseType` / `dbType` / `dialect` / `type` / `driver` 等候选字段里**是否包含 `opcua`** 来判断；
2. 识别不出 OPC UA 连接时**绝不自动选中任何连接**，只提示「未发现 OPC UA 连接」；
3. 下拉框里非 OPC UA 的连接会加 `⚠ 非 OPC UA ·` 前缀，手动选中时也会再警告一次。

另外，工作台 `context` 里的 `connectionId`（宿主打开本工作台时传入）优先级最高，优先于列表推断。
