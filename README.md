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
├── assets/                    # plugin.svg（官方 OPC UA wordmark，512×512）
├── ui/                        # 沙箱工作台（index.html、styles.css、app.js、icon.svg）
├── backend/                   # Rust sidecar（Cargo.toml、src/main.rs）
├── scripts/                   # build-store-candidate.mjs（生成商店候选元数据）
├── store/                     # 上架 DBX Store 用的材料（publisher 记录、候选元数据、PR 正文）
├── .github/workflows/         # plugin-release.yml（上游 reusable workflow，产出多平台包）
├── LICENSE                    # MIT
└── dist/                      # 打包输出（不提交 Git）
```

### 图标

图标直接用 OPC UA 官方素材，与 `dbx-plugin-modbus` 保持一致（宿主支持 SVG 图标）：

- `assets/plugin.svg`（512×512）：官方 wordmark SVG，`manifest.json` 的根节点 `icon`、
  connection-provider 与 workbench 的 `icon` 都指向它；商店候选元数据的 `icon` 指向仓库同路径文件。
- `ui/icon.svg`：同一份 SVG，但 `viewBox` 收到 `24 204 464 104`（wordmark 实际占位），
  供工作台标题栏按 96×22 显示。官方图是 4.48:1 的横条，套进 36px 方框里只剩约 7px 高，
  所以标题栏用宽幅 lockup 而不是方块。

来源是 `dbx-official-center-only-svg/dbx-opcua-official-center-only.svg`（512 画布里居中放一张
448×100 的透明 PNG，带 `feDropShadow`）。

> 之前两版自绘图标（渐变底板 + OPC/UA 字样）已弃用，相关脚本保留为备用工具：
> `scripts/official_icon.py`（从官方 SVG 生成带底板的 PNG，深色侧栏下更安全）、
> `scripts/draw_icon.py`（纯几何 SDF 绘制，不依赖官方素材）、`scripts/refit_icons.py --check`
> （量图形在画布中的占比）。若哪天宿主不支持 SVG，用 `official_icon.py` 生成 PNG 并把
> `manifest.json` 里的三处 `.svg` 改回 `.png` 即可。

历史：最初是浅底横排 "OPC UA" 文字（只占板高 1/3，小尺寸又小又糊），已全部被这一版替代。

## 打包

使用官方 CLI（Node 包，内含预编译二进制）：

```bash
npm install --global @dbx-app/plugin-cli
dbx-plugin package .                     # 为当前平台构建并打包
dbx-plugin package . --target windows-x64 --output-dir dist
```

产物（未签名审核候选包）：

```
dist/io.github.yuzhi.opcua-0.2.0-<target>.dbxp
dist/io.github.yuzhi.opcua-0.2.0-<target>.artifact.json
```

### 升级版本号（必须三处同步）

后端 sidecar 在 `plugin/initialize` 时会上报身份 `id/version`，宿主要求它与 `manifest.json` 完全一致，
否则启动即报 `Plugin backend identity '<id>/<old>' does not match manifest '<id>/<new>'`。

1. `manifest.json`：`version` 与 `source`（`.../tree/vX.Y.Z`）；
2. `backend/Cargo.toml`：`version` —— `src/main.rs` 里 `PLUGIN_VERSION` 用的是
   `env!("CARGO_PKG_VERSION")`，改这里即可，不要再手写常量；
3. `backend/Cargo.lock`：CLI 以 `--locked` 构建，不同步会报 `cannot update the lock file`。

`store/candidates/*.json` 的 `version` / `icon` / `source` 由 `scripts/build-store-candidate.mjs` 生成。

### 版本命名规范（vx.x.x 用在哪、不用在哪）

| 位置 | 格式 | 原因 |
|------|------|------|
| `manifest.json` / `Cargo.toml` / `Cargo.lock` / 包内元数据 | `X.Y.Z` | 纯 semver。宿主启动时校验 sidecar 上报的身份与 manifest 完全一致，Cargo 也不接受 `v` 前缀，**这里带 `v` 会直接启动失败** |
| Git 标签、Release 标题、`.dbx-store.json` 的 `releaseNotes` 首行 | `vX.Y.Z` | 面向人的展示名 |
| 产物文件名（`.dbxp` / `.artifact.json`） | `<plugin-id>-X.Y.Z-<target>.dbxp` | `dbx-plugin` CLI 默认命名，**不要改名**：商店候选 URL 与签名工作流的逐字节校验都按这个名字生成 |

即：包内与产物一律 `0.2.3`，凡是「说给人听」的地方统一 `v0.2.3`。

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
git tag v0.2.0 && git push origin v0.2.0
```

在 GitHub 上发布该 Tag 的 Release，`plugin-release.yml` 会自动为 5 个平台各构建一个 `.dbxp`，并把包与汇总的 `release-candidates.json` 上传到该 Release。

### 2. 生成候选元数据

```bash
# 仅本地已构建的平台（默认读取 dist/*.artifact.json）
node scripts/build-store-candidate.mjs --repo bobocha214/dbx-plugin-opcua --tag v0.2.0

# Release 已发布、拿到全部平台后（下载 Release 里的 release-candidates.json）
node scripts/build-store-candidate.mjs --repo bobocha214/dbx-plugin-opcua --tag v0.2.0 \
  --release-candidates ./release-candidates.json
```

脚本从**真实构建产物**读取 SHA-256 与大小写入 `store/candidates/io.github.yuzhi.opcua.json`，不要手抄这两个值——签名工作流会逐字节比对，不一致直接拒绝。重新打包后哈希会变，必须重新生成。

> 汇总文件不会自动识别：只有显式传 `--release-candidates` 才会使用。因为一份与实际上传字节不符的陈旧清单会 pin 错哈希。

商店展示文案（name / description / tags / license / localizations / releaseNotes）取自 `.dbx-store.json`——这也是商店同步器读取的同一份元数据。要改文案就改那里再重跑脚本，不要直接手改 `store/candidates/*.json`（下次生成会被覆盖）。

> `store/make-candidate.mjs` 是功能等价的早期脚本，但它**必须**传 `--release-candidates`（只适用于 Release 已发布之后）。本地打包完就想生成候选时用 `scripts/` 下这个。

### 3. 提交 catalog PR

fork `t8y2/dbx-store`，在 PR 中加入两个文件（**只加这两个**）：

| 本仓库文件 | 放到 dbx-store 的 |
|---|---|
| `store/publishers/yuzhi.json` | `publishers/yuzhi.json` |
| `store/candidates/io.github.yuzhi.opcua.json` | `candidates/io.github.yuzhi.opcua.json` |

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

### 界面布局（三栏）
- **顶部**：状态条。左侧是插件名，右侧显示**当前连接名 + 连接状态**（宿主打开工作区时已指定连接），「切换」按钮展开折叠面板，里面才是连接下拉与手动 `连接 ID`。连接由宿主管理，UI 不自行发起连接。
- **左**：地址空间树（窄栏，默认 236px），加载三个标准根 `Objects` (`i=85`)、`Types` (`i=86`)、`Views` (`i=87`)；按 `HierarchicalReferences` 前向展开，自动翻页 continuation point。
- **中**：点位区。把左侧树上的 `Variable` 点位**拖动**到这里即加入；拖入 `Object` / `Method` 等非 `Variable` 节点会被拒绝并提示原因。
- **右**：选中点位的详细信息（NodeId / BrowseName / DisplayName / NodeClass）、当前值（可「读取」）、整行写入表单，以及加入/移出点位区。
- 点击左侧树节点或中间点位卡片都能选中该点位，树行与卡片高亮联动，右侧详情随之切换。左侧树没有展开过的点位（例如从 `localStorage` 恢复的）同样可以查看详情。
- **分栏宽度可拖动**：两条分隔条用鼠标拖动即可调整左/中/右三栏宽度（键盘聚焦后 `←` `→` 微调，`Shift` 加速），双击分隔条恢复默认 236 / 302，宽度按面板记在 `localStorage`；面板变窄时中间点位区保底 200px。
- 把点位拖到**右侧详情区**也能直接查看该点位详情。

### 点位区（持续观察 + 就地改写下发）
- 加入点位区的三种方式（只有 `Variable` 可加入）：
  1. **拖拽**：按住树上的点位行拖到中点位区（**只用 pointer 事件实现**：树行 `draggable=false`，
     因为原生 HTML5 拖拽在 WebView2 里会先发 `pointercancel` 掐断拖拽、而它自己的 `drop` 又不送达，
     两头落空；命中判定用面板矩形而非 `elementFromPoint`；无 `PointerEvent` 的环境自动退回鼠标事件）；
  2. **＋ 按钮**：树上 `Variable` 行右侧常驻的「＋」；
  3. **双击**树上 `Variable` 行，或右侧详情点「加入点位区」。
- 观察：点「▶ 持续观察」，按 200ms / 500ms / 1s / 2s / 5s 间隔轮询；走 **`opcua/readMany` 一次批量读**，N 个点只花一次往返。上一轮未返回时自动跳过本轮，避免堆积。
- 改值下发：**点数值**（或卡片上的 ✎）→ 卡片展开一行写入区（整行输入框 + 类型下拉）→ 改完**点「写入」按钮**才 `opcua/write` 下发；回车等同于点按钮，`Esc` / 「取消」放弃。**失焦不再自动写入**（旧行为会在轮询或点击抢走焦点时误下发）。
- 写入过程：按钮变「写入中…」并禁用，成功后关闭写入区、自动回读该点、单元格绿色闪烁；失败保留写入区与已填内容，红色闪烁并提示服务器状态码与原因，可直接改完再点。
- 轮询期间**不会覆盖你正在编辑的卡片**；写入的数据类型由 sidecar 读取该节点的 `DataType` 属性决定（见下节），不依赖本地猜测。
- 点位区按连接 id 存 `localStorage`，切换连接后自动恢复；`×` 移除，「清空」全清。
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
| `opcua/write` | `{ id, nodeId, dataType?, value }` | 写标量值，返回 `{ success, status, statusCode, dataType, message }` |

连接 id 同时接受 `id`、`connectionId` 与嵌套 `connection.id`。`nodeIds` 同时接受 `["i=85"]` 与 `[{"nodeId":"i=85"}]`。

### 写入的判定逻辑

写入不是「发出去就算成功」，`opcua/write` 的行为如下：

1. **先用一次读取拿到服务器的事实**：`DataType`（14）、`ValueRank`（15）、`UserAccessLevel`（18）。`ValueRank` 为数组/多维数组时直接报错，不再让服务器回一个含糊的 `BadTypeMismatch`。
2. **类型以服务器为准**：`dataType: "Auto"`（UI 默认）或手动指定的类型只在服务器 `DataType` 是抽象类型/自定义结构时才生效。手动选一个与服务器不一致的类型不会「覆盖」它 —— 那样只会得到 `BadTypeMismatch`。响应里的 `dataTypeSource` 为 `server` 或 `request`。
3. **值的解析对人宽容**：`Boolean` 接受 `true/false`、`1/0`、`yes/no`、`on/off`；数字接受前后空格、单个小数逗号（`1,5`）与 JSON 数字（`value: 12.5`，不限于字符串）；越界会说明是哪一类越界。
4. **状态码即结论**：只有 `Good` 系列才 `success: true`。`BadNotWritable` / `BadUserAccessDenied` / `BadOutOfRange` 等一律 `success: false`，并附中文原因（`message`），UI 用红色显示，不再把失败渲染成「已写入」。
5. **回读校验**：写成功后立即回读，若值没变（只读点位、被 PLC 程序覆盖、服务器忽略写入）会显式提示，而不是假装成功。

### 连接时机

`connection/connect` 会等到 OPC UA 握手（SecureChannel + `CreateSession`）真正完成才返回成功。此前该调用在握手开始前就返回，紧接着发出的第一条读/写请求会概率性失败于 `BadNotConnected: Unable to send request, transport is not connected`；`opcua/*` 方法现在也会等待会话就绪（自动重连窗口内同样适用），超时设为 10s。

## 安全：TLS 与凭据

- **默认校验证书**。连接表单新增「信任任意服务器证书（不安全）」开关，**默认关闭**。关闭时客户端会按 OPC UA 信任库校验服务器证书（链与主机名）；自签证书的设备需在连接表单里显式勾选该开关，或把服务器证书导入信任库。
- **不再默认信任任何证书**。早期版本硬编码了 `trust_server_certs(true)`，会接受任意服务器证书且端点安全策略由服务器决定（可能协商到明文 `SecurityPolicy#None`），在「用户名 / 密码」认证下凭据可被中间人截获。0.2.0 起已移除该默认行为。
- **客户端密钥**。客户端会生成自签的应用实例密钥对，但写入位于系统临时目录（`dbx-opcua-sidecar-pki`），**不会写入插件安装目录**，且每次运行重新生成。
- **网络直连**。连接会直接连到你填写的 `opc.tcp://主机:端口`（默认 4840），无代理 / 中转之外的外部请求。
- **写入按需下发**。点位区/详情面板改值后，只有点击「写入」按钮才会 `opcua/write`（写后有回读校验）；请确认数值无误再点。改值本身不会自动生效，失焦也不会。

## 待替换 / 已知限制

- 插件身份：`id` = `io.github.yuzhi.opcua`，`publisher` = `yuzhi`，仓库位于 `https://github.com/bobocha214/dbx-plugin-opcua`（publisher 与 GitHub 账号的显示名一致）。改动 `id` / `publisher` / `version` 中任何一项都会改变包内容，需重新打包并重新生成候选元数据。
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
  "plugin": { "id": "io.github.yuzhi.opcua", "version": "0.2.0" }
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
