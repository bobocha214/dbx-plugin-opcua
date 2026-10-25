(function () {
  "use strict";

  // Identity — must match manifest.json. Used to tell OPC UA connections apart from
  // other plugins' connections (e.g. a Modbus device) in the host's connection list.
  var PLUGIN_ID = "io.github.bobocha214.opcua";
  var PROVIDER_ID = "io.github.bobocha214.opcua.connection";
  var DB_TYPE = "opcua";

  var WATCH_KEY_PREFIX = "dbx-opcua-watch:";

  // ----------------------------------------------------------------------
  // DBX Host API bridge.
  // The connection itself is managed by DBX (connection-provider); the UI never sees
  // host/port/credentials — getConnections() returns a desensitized summary only.
  // ----------------------------------------------------------------------
  function invoke(method, params, timeoutMs) {
    if (!window.dbxPlugin || typeof window.dbxPlugin.invoke !== "function") {
      return Promise.reject(
        new Error("window.dbxPlugin.invoke 不可用：请在 DBX 宿主中运行此界面。")
      );
    }
    return window.dbxPlugin.invoke(method, params || {}, { timeoutMs: timeoutMs || 15000 });
  }

  // Some hosts return the full JSON-RPC envelope; others return result directly.
  function unwrap(res) {
    if (res && typeof res === "object" && Object.prototype.hasOwnProperty.call(res, "result")) {
      return res.result;
    }
    return res;
  }

  function msg(e) {
    if (!e) return "未知错误";
    if (typeof e === "string") return e;
    return e.message || String(e);
  }

  // ----------------------------------------------------------------------
  // State
  // ----------------------------------------------------------------------
  var state = {
    connectionId: "",
    connections: [], // all connections reported by the host
    nodes: {},       // nodeId -> { data, expanded, loaded, li, childUl }
    watch: [],       // [{ nodeId, displayName, browseName, dataType, value, status, sourceTimestamp }]
    watchEls: {},    // nodeId -> { row, v, dt }
    lasts: {},       // nodeId -> last read result (also for points not in the watch list)
    selected: "",    // nodeId currently shown in the right-hand detail pane
    dragNodeId: "",  // nodeId being dragged out of the tree
    dtDirty: false,  // user picked a write data type by hand -> stop auto-following
    polling: false,
    tickBusy: false,
    timer: null,
  };

  var $ = function (id) { return document.getElementById(id); };
  var tree = $("tree");
  var detail = $("detail");
  var watchList = $("watchList");
  var boardPane = $("boardPane");
  var dropHint = $("dropHint");

  function setStatus(text, cls) {
    var el = $("status");
    el.textContent = text;
    el.className = "status" + (cls ? " " + cls : "");
  }

  function toast(t, isErr) {
    var el = $("toast");
    el.textContent = t;
    el.className = "toast show" + (isErr ? " err" : "");
    setTimeout(function () { el.className = "toast"; }, 2800);
  }

  function esc(s) {
    return String(s === null || s === undefined ? "" : s)
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function fmtValue(v) {
    if (v === null || v === undefined) return "(null)";
    if (typeof v === "object") return JSON.stringify(v);
    return String(v);
  }

  // ----------------------------------------------------------------------
  // Connections
  //
  // getConnections() returns a *desensitized* summary across ALL plugins. Per the
  // plugin SDK the summary shape is roughly { id, providerId, name, label } — it has
  // no dbType/dialect field, so a naive `dbType === "opcua"` filter never matches and
  // silently falls back to conns[0], which is how a Modbus device ended up selected.
  // We match on any id/type-ish field containing "opcua", and never auto-select a
  // connection we could not identify as OPC UA.
  // ----------------------------------------------------------------------
  function connMatches(c) {
    if (!c) return false;
    var candidates = [
      c.providerId, c.provider_id, c.pluginId, c.plugin_id,
      c.connectionType, c.connection_type,
      c.databaseType, c.database_type,
      c.dbType, c.db_type,
      c.dialect, c.type, c.driver,
    ];
    for (var i = 0; i < candidates.length; i++) {
      var v = candidates[i];
      if (typeof v === "string" && v.toLowerCase().indexOf(DB_TYPE) >= 0) return true;
    }
    return false;
  }

  async function loadConnections() {
    try {
      var caps = window.dbxPlugin.capabilities || {};
      if (!caps.connectionsApi) return [];
      var list = null;
      if (typeof window.dbxPlugin.getConnections === "function") {
        list = await window.dbxPlugin.getConnections();
      } else if (
        window.dbxPlugin.connections &&
        typeof window.dbxPlugin.connections.list === "function"
      ) {
        list = await window.dbxPlugin.connections.list();
      }
      return Array.isArray(list) ? list : [];
    } catch (e) {
      return [];
    }
  }

  function renderConnections(conns) {
    var sel = $("connSelect");
    sel.innerHTML = "";

    var opcua = conns.filter(connMatches);
    var shown = opcua.length ? opcua : conns;

    if (!shown.length) {
      var o0 = document.createElement("option");
      o0.value = "";
      o0.textContent = "（宿主未暴露连接列表，请手动填写 ID）";
      sel.appendChild(o0);
      return;
    }

    if (!opcua.length) {
      var warn = document.createElement("option");
      warn.value = "";
      warn.textContent = "（未发现 OPC UA 连接，以下为其他插件的连接）";
      sel.appendChild(warn);
    }

    shown.forEach(function (c) {
      var o = document.createElement("option");
      o.value = c.id;
      var tag = c.connected || c.isConnected ? " · 已连接" : "";
      var prefix = connMatches(c) ? "" : "⚠ 非 OPC UA · ";
      o.textContent = prefix + (c.name || c.label || c.id) + tag;
      sel.appendChild(o);
    });
  }

  async function useConnection(id) {
    if (!id) { toast("请提供连接 ID", true); return; }
    var known = null;
    for (var i = 0; i < state.connections.length; i++) {
      if (state.connections[i].id === id) { known = state.connections[i]; break; }
    }
    if (known && !connMatches(known)) {
      toast("该连接不是 OPC UA 连接（" + (known.name || known.label || id) + "），浏览可能失败", true);
    }

    state.connectionId = id;
    state.nodes = {};
    state.lasts = {};
    state.selected = "";
    tree.innerHTML = "";
    detail.innerHTML = '<p class="hint">在左侧树或中间点位区选择一个点位查看详情。</p>';

    loadWatch(id);
    renderWatch();

    var label = known ? (known.name || known.label || id) : id;
    setStatus("已选择 " + label, "ok");
    await loadRoots();
    if (state.polling) startPolling();
  }

  // ----------------------------------------------------------------------
  // Tree
  // ----------------------------------------------------------------------
  async function loadRoots() {
    if (!state.connectionId) return;
    try {
      var res = unwrap(await invoke("opcua/root", { id: state.connectionId }));
      var roots = (res && res.roots) || [];
      tree.innerHTML = "";
      roots.forEach(function (r) { addNode(r, tree); });
    } catch (e) {
      setStatus("加载失败", "err");
      toast("加载根节点失败：" + msg(e), true);
    }
  }

  function addNode(node, parentUl) {
    state.nodes[node.nodeId] = {
      data: node, expanded: false, loaded: false, li: null, childUl: null,
    };

    var li = document.createElement("li");
    var row = document.createElement("div");
    row.className = "node-row";
    row.dataset.nodeId = node.nodeId;

    // Points are dragged from the tree into the middle board. Every node is
    // draggable so the gesture always starts; the drop handler accepts only
    // Variables and explains why anything else was rejected.
    row.draggable = true;
    row.addEventListener("dragstart", function (ev) {
      state.dragNodeId = node.nodeId;
      row.classList.add("dragging");
      if (ev.dataTransfer) {
        ev.dataTransfer.effectAllowed = "copy";
        try { ev.dataTransfer.setData("text/plain", node.nodeId); } catch (e) { /* ignore */ }
      }
    });
    row.addEventListener("dragend", function () {
      state.dragNodeId = "";
      row.classList.remove("dragging");
    });

    var tw = document.createElement("span");
    tw.className = "twisty";
    tw.textContent = "▸";

    var name = document.createElement("span");
    name.className = "node-name";
    name.textContent = node.displayName || node.browseName || node.nodeId;

    var badge = document.createElement("span");
    badge.className = "badge " + (node.nodeClass || "");
    badge.textContent = node.nodeClass || "";

    var id = document.createElement("span");
    id.className = "node-id";
    id.textContent = node.nodeId;

    row.appendChild(tw);
    row.appendChild(name);
    row.appendChild(badge);
    row.appendChild(id);

    // Variables are the writable "points" — offer a quick add-to-watch affordance.
    if (node.nodeClass === "Variable") {
      var add = document.createElement("button");
      add.className = "watch-add";
      add.textContent = "+";
      add.title = "加入监视列表";
      add.dataset.nodeId = node.nodeId;
      add.addEventListener("click", function (ev) {
        ev.stopPropagation();
        addWatch(node);
      });
      row.appendChild(add);
    }

    row.addEventListener("click", function (ev) {
      ev.stopPropagation();
      selectNode(node.nodeId);
      toggleExpand(node.nodeId);
    });

    li.appendChild(row);

    var childUl = document.createElement("ul");
    childUl.style.display = "none";
    li.appendChild(childUl);

    state.nodes[node.nodeId].li = li;
    state.nodes[node.nodeId].childUl = childUl;

    parentUl.appendChild(li);
    syncWatchButtons();
  }

  async function toggleExpand(nodeId) {
    var node = state.nodes[nodeId];
    if (!node) return;
    var tw = node.li.querySelector(".twisty");

    if (node.expanded) {
      node.childUl.style.display = "none";
      tw.textContent = "▸";
      node.expanded = false;
      return;
    }

    if (!node.loaded) {
      try {
        var res = unwrap(
          await invoke("opcua/browse", { id: state.connectionId, nodeId: nodeId })
        );
        var children = (res && res.children) || [];
        children.forEach(function (c) { addNode(c, node.childUl); });
        node.loaded = true;
      } catch (e) {
        toast("浏览失败：" + msg(e) + "（若提示未连接，请先在 DBX 中打开该连接）", true);
        return;
      }
    }
    node.childUl.style.display = "block";
    tw.textContent = "▾";
    node.expanded = true;
  }

  // ----------------------------------------------------------------------
  // Selection — drives the tree highlight, the board highlight and the detail pane
  // ----------------------------------------------------------------------
  function selectNode(nodeId) {
    state.selected = nodeId || "";
    highlight(nodeId);
    renderDetail(nodeId);
  }

  function highlight(nodeId) {
    var prev = tree.querySelector(".node-row.selected");
    if (prev) prev.classList.remove("selected");
    var node = state.nodes[nodeId];
    if (node) node.li.querySelector(".node-row").classList.add("selected");

    Object.keys(state.watchEls).forEach(function (id) {
      var el = state.watchEls[id];
      if (el && el.row) el.row.classList.toggle("selected", id === nodeId);
    });
  }

  // Identity for the detail pane. Works for points restored from storage whose
  // tree row has never been browsed (state.nodes has no entry for them).
  function nodeInfo(nodeId) {
    var n = state.nodes[nodeId];
    var w = findItem(nodeId);
    var d = (n && n.data) || {};
    return {
      nodeId: nodeId,
      browseName: d.browseName || (w && w.browseName) || "",
      displayName: d.displayName || (w && w.displayName) || nodeId,
      nodeClass: d.nodeClass || (w ? "Variable" : ""),
    };
  }

  function lastDt(nodeId) {
    if (state.lasts[nodeId] && state.lasts[nodeId].dataType) return state.lasts[nodeId].dataType;
    var w = findItem(nodeId);
    return (w && w.dataType) || "";
  }

  // ----------------------------------------------------------------------
  // Detail panel
  // ----------------------------------------------------------------------
  function valueBoxHtml(res) {
    if (!res) return '<span class="hint">尚未读取。点击“读取”获取当前值。</span>';
    var val = res.value === null || res.value === undefined ? "(null)" : res.value;
    if (typeof val === "object") val = JSON.stringify(val);
    return (
      '<div class="big">' + esc(String(val)) + "</div>" +
      '<div class="meta">类型: ' + esc(res.dataType || "") +
      " · 状态: " + esc(res.status || "") +
      (res.sourceTimestamp ? " · 时间: " + esc(res.sourceTimestamp) : "") +
      "</div>"
    );
  }

  function refreshValueBox(nodeId) {
    var box = $("valueBox");
    if (!box || state.selected !== nodeId) return;
    box.innerHTML = valueBoxHtml(state.lasts[nodeId]);
    syncDt(nodeId);
  }

  // Follow the data type the server reports, unless the user picked one by hand.
  function syncDt(nodeId) {
    if (state.dtDirty) return;
    var sel = $("dt");
    var dt = lastDt(nodeId);
    if (!sel || !dt) return;
    for (var i = 0; i < sel.options.length; i++) {
      if (sel.options[i].value === dt) { sel.value = dt; return; }
    }
  }

  function renderDetail(nodeId) {
    if (!nodeId) {
      detail.innerHTML = '<p class="hint">在左侧树或中间点位区选择一个点位查看详情。</p>';
      return;
    }
    var info = nodeInfo(nodeId);
    var watched = !!findItem(nodeId);
    var isVar = info.nodeClass === "Variable";
    state.dtDirty = false;

    var html = '<div class="kv">';
    html += '<div class="k">NodeId</div><div class="v mono">' + esc(info.nodeId) + "</div>";
    html += '<div class="k">BrowseName</div><div class="v">' + esc(info.browseName) + "</div>";
    html += '<div class="k">DisplayName</div><div class="v">' + esc(info.displayName) + "</div>";
    html += '<div class="k">NodeClass</div><div class="v">' + esc(info.nodeClass) + "</div>";
    html += "</div>";

    if (!isVar) {
      detail.innerHTML = html + '<p class="hint">只有 Variable 点位可以读取与写入。</p>';
      return;
    }

    html += '<button id="watchBtn" class="watch-btn">' +
      (watched ? "移出点位区" : "加入点位区") + "</button>";
    html += '<div class="section-label">当前值</div>';
    html += '<div class="value-box" id="valueBox">' + valueBoxHtml(state.lasts[nodeId]) + "</div>";
    html += '<button id="readBtn">读取</button>';

    html += '<div class="section-label">写入</div>';
    html +=
      '<div class="write-form">' +
      '<select id="dt">' + dtypeOptions(lastDt(nodeId)) + "</select>" +
      '<input id="wv" type="text" placeholder="值" />' +
      '<button id="writeBtn">写入</button>' +
      "</div>";
    html += '<div id="writeRes" class="hint" style="margin-top:8px;"></div>';
    detail.innerHTML = html;

    $("watchBtn").addEventListener("click", function () {
      if (findItem(nodeId)) { removeWatch(nodeId); } else { addWatch(info); }
    });
    $("readBtn").addEventListener("click", function () { readValue(nodeId); });
    $("writeBtn").addEventListener("click", function () { writeValue(nodeId); });
    $("dt").addEventListener("change", function () { state.dtDirty = true; });
    $("wv").addEventListener("keydown", function (e) {
      if (e.key === "Enter") { e.preventDefault(); writeValue(nodeId); }
    });
    syncDt(nodeId);
  }

  function dtypeOptions(selected) {
    var types = [
      "Boolean", "SByte", "Byte", "Int16", "UInt16", "Int32", "UInt32",
      "Int64", "UInt64", "Float", "Double", "String",
    ];
    return types
      .map(function (t) {
        return '<option value="' + t + '"' + (t === selected ? " selected" : "") + ">" + t + "</option>";
      })
      .join("");
  }

  async function readValue(nodeId) {
    try {
      var res = unwrap(await invoke("opcua/read", { id: state.connectionId, nodeId: nodeId })) || {};
      state.lasts[nodeId] = res;
      var item = findItem(nodeId);
      if (item) {
        item.value = res.value;
        item.dataType = res.dataType || item.dataType;
        item.status = res.status || "";
        item.sourceTimestamp = res.sourceTimestamp || "";
        paintItem(nodeId);
      }
      refreshValueBox(nodeId);
    } catch (e) {
      var box = $("valueBox");
      if (box) box.innerHTML = '<span class="hint">读取失败：' + esc(msg(e)) + "</span>";
    }
  }

  async function writeValue(nodeId) {
    var dt = $("dt").value;
    var v = $("wv").value;
    var out = $("writeRes");
    try {
      var res = unwrap(
        await invoke("opcua/write", {
          id: state.connectionId, nodeId: nodeId, dataType: dt, value: v,
        })
      );
      out.textContent = "写入结果：" + ((res && res.status) || "未知");
      out.style.color = "var(--ok)";
      toast("已写入 " + nodeId + " = " + v);
      readValue(nodeId); // read back so the pane and the board show what the server took
    } catch (e) {
      out.textContent = "写入失败：" + msg(e);
      out.style.color = "var(--err)";
    }
  }

  // ----------------------------------------------------------------------
  // Drag & drop — points are dragged out of the tree into the middle board
  // ----------------------------------------------------------------------
  function bindDrop() {
    boardPane.addEventListener("dragover", function (ev) {
      ev.preventDefault(); // required, or the drop event never fires
      if (ev.dataTransfer) ev.dataTransfer.dropEffect = "copy";
      boardPane.classList.add("drop-active");
    });
    boardPane.addEventListener("dragleave", function (ev) {
      if (!boardPane.contains(ev.relatedTarget)) boardPane.classList.remove("drop-active");
    });
    boardPane.addEventListener("drop", function (ev) {
      ev.preventDefault();
      boardPane.classList.remove("drop-active");

      var nodeId = state.dragNodeId;
      if (!nodeId && ev.dataTransfer) {
        try { nodeId = ev.dataTransfer.getData("text/plain"); } catch (e) { nodeId = ""; }
      }
      if (!nodeId) return;

      var node = state.nodes[nodeId] ? state.nodes[nodeId].data : null;
      if (node && node.nodeClass && node.nodeClass !== "Variable") {
        toast("只能拖入 Variable 点位（" + node.nodeClass + " 不可读取/写入）", true);
        return;
      }
      addWatch(node || { nodeId: nodeId, displayName: nodeId, nodeClass: "Variable" });
    });
  }

  // ----------------------------------------------------------------------
  // Watch list
  // ----------------------------------------------------------------------
  function findItem(nodeId) {
    for (var i = 0; i < state.watch.length; i++) {
      if (state.watch[i].nodeId === nodeId) return state.watch[i];
    }
    return null;
  }

  function addWatch(node) {
    if (!node || !node.nodeId) return;
    if (node.nodeClass && node.nodeClass !== "Variable") {
      toast("只有 Variable 点位可以加入点位区", true);
      return;
    }
    if (findItem(node.nodeId)) { toast("该点位已在点位区中"); return; }
    state.watch.push({
      nodeId: node.nodeId,
      displayName: node.displayName || node.browseName || node.nodeId,
      browseName: node.browseName || "",
      dataType: "",
      value: null,
      status: "",
      sourceTimestamp: "",
    });
    saveWatch();
    renderWatch();
    syncWatchButtons();
    selectNode(node.nodeId); // dropping a point selects it: the detail pane follows
    if (state.polling) tick();
    toast("已加入点位区：" + (node.displayName || node.nodeId));
  }

  function removeWatch(nodeId) {
    state.watch = state.watch.filter(function (i) { return i.nodeId !== nodeId; });
    saveWatch();
    renderWatch();
    syncWatchButtons();
    if (state.selected === nodeId) renderDetail(nodeId); // refresh the join/leave button
  }

  function syncWatchButtons() {
    Object.keys(state.nodes).forEach(function (nodeId) {
      var n = state.nodes[nodeId];
      var btn = n.li && n.li.querySelector(".watch-add");
      if (!btn) return;
      if (findItem(nodeId)) btn.classList.add("on");
      else btn.classList.remove("on");
    });
    var c = $("watchCount");
    if (c) c.textContent = String(state.watch.length);
  }

  function renderWatch() {
    watchList.innerHTML = "";
    state.watchEls = {};
    var count = $("watchCount");
    if (count) count.textContent = String(state.watch.length);

    // The empty state lives outside the list so it can double as the drop hint.
    dropHint.classList.toggle("hidden", state.watch.length > 0);
    if (!state.watch.length) return;

    state.watch.forEach(function (item) {
      var row = document.createElement("div");
      row.className = "watch-item" + (state.selected === item.nodeId ? " selected" : "");
      row.dataset.nodeId = item.nodeId;
      // Clicking a card selects the point -> its details show in the right column.
      row.addEventListener("click", function () { selectNode(item.nodeId); });

      var main = document.createElement("div");
      main.className = "wi-main";
      var nm = document.createElement("div");
      nm.className = "wi-name";
      nm.textContent = item.displayName;
      nm.title = item.displayName;
      var id = document.createElement("div");
      id.className = "wi-id";
      id.textContent = item.nodeId;
      id.title = item.nodeId;
      main.appendChild(nm);
      main.appendChild(id);

      var valWrap = document.createElement("div");
      valWrap.className = "wi-val";

      var v = document.createElement("span");
      v.className = "wi-v stale";
      v.textContent = fmtValue(item.value);
      v.title = "点击直接修改，回车下发写入";
      v.addEventListener("click", function (ev) { ev.stopPropagation(); beginEdit(item.nodeId); });

      var dt = document.createElement("span");
      dt.className = "wi-dt";
      dt.textContent = item.dataType || "";

      valWrap.appendChild(v);
      valWrap.appendChild(dt);

      var del = document.createElement("button");
      del.className = "wi-del";
      del.textContent = "×";
      del.title = "移除";
      del.addEventListener("click", function (ev) { ev.stopPropagation(); removeWatch(item.nodeId); });

      row.appendChild(main);
      row.appendChild(valWrap);
      row.appendChild(del);
      watchList.appendChild(row);

      state.watchEls[item.nodeId] = { row: row, v: v, dt: dt };
    });
  }

  // Repaint one board cell from the freshest data we have for that node.
  function paintItem(nodeId) {
    var el = state.watchEls[nodeId];
    if (!el || el.v.classList.contains("editing")) return;
    var item = findItem(nodeId);
    var last = state.lasts[nodeId] || {};
    var status = (item && item.status) || last.status || "";
    var ts = (item && item.sourceTimestamp) || last.sourceTimestamp || "";
    var name = (item && item.displayName) || nodeId;
    el.v.textContent = fmtValue(item ? item.value : last.value);
    el.v.classList.remove("stale");
    el.dt.textContent = (item && item.dataType) || last.dataType || "";
    el.v.title =
      name +
      (status ? " · 状态: " + status : "") +
      (ts ? " · 时间: " + ts : "") +
      "\n点击直接修改，回车下发写入";
  }

  function applyValues(values) {
    values.forEach(function (r) {
      state.lasts[r.nodeId] = r;
      var item = findItem(r.nodeId);
      if (item) {
        item.value = r.value;
        item.dataType = r.dataType || item.dataType;
        item.status = r.status || "";
        item.sourceTimestamp = r.sourceTimestamp || "";
      }
      paintItem(r.nodeId);
    });
    if (state.selected) refreshValueBox(state.selected);
  }

  function flash(nodeId, ok) {
    var el = state.watchEls[nodeId];
    if (!el) return;
    el.v.classList.remove("flash-ok", "flash-err");
    el.v.classList.add(ok ? "flash-ok" : "flash-err");
    setTimeout(function () { el.v.classList.remove("flash-ok", "flash-err"); }, 700);
  }

  // Click the value -> it becomes a selected input; Enter (or blur) writes it down.
  function beginEdit(nodeId) {
    var item = findItem(nodeId);
    var el = state.watchEls[nodeId] && state.watchEls[nodeId].v;
    if (!item || !el || el.classList.contains("editing")) return;

    var current = item.value === null || item.value === undefined ? "" : String(item.value);

    var input = document.createElement("input");
    input.className = "wi-edit";
    input.value = current;

    el.textContent = "";
    el.classList.add("editing");
    el.appendChild(input);
    input.focus();
    input.select(); // "鼠标直接选中对应值"

    var done = false;
    function finish(commit) {
      if (done) return;
      done = true;
      el.classList.remove("editing");
      var next = input.value;
      el.textContent = fmtValue(item.value);
      if (commit && next !== current) {
        writeItem(nodeId, next);
      }
    }

    input.addEventListener("keydown", function (e) {
      if (e.key === "Enter") { e.preventDefault(); finish(true); }
      else if (e.key === "Escape") { e.preventDefault(); finish(false); }
    });
    input.addEventListener("blur", function () { finish(true); });
  }

  async function writeItem(nodeId, newVal) {
    var item = findItem(nodeId);
    if (!item) return;
    var dt = lastDt(nodeId);
    if (!dt || dt === "<null>" || dt === "<unsupported>") dt = "String";

    try {
      await invoke(
        "opcua/write",
        { id: state.connectionId, nodeId: nodeId, dataType: dt, value: newVal },
        10000
      );
      flash(nodeId, true);
      toast("已写入 " + (item.displayName || nodeId) + " = " + newVal);
    } catch (e) {
      flash(nodeId, false);
      toast("写入失败：" + msg(e), true);
      return;
    }

    // Read back so the cell shows what the server actually accepted.
    try {
      var r = unwrap(
        await invoke("opcua/readMany", { id: state.connectionId, nodeIds: [nodeId] }, 10000)
      );
      applyValues((r && r.values) || []);
    } catch (e) { /* non-fatal: next poll will refresh */ }
  }

  // ----------------------------------------------------------------------
  // Polling
  // ----------------------------------------------------------------------
  function pollMs() {
    var v = parseInt($("pollMs").value, 10);
    return isNaN(v) ? 1000 : v;
  }

  function startPolling() {
    if (!state.watch.length) { toast("先添加监视点", true); return; }
    stopTimer();
    state.polling = true;
    var btn = $("pollBtn");
    btn.textContent = "⏸ 停止观察";
    btn.classList.add("running");
    state.timer = setInterval(tick, pollMs());
    tick();
  }

  function stopPolling() {
    stopTimer();
    state.polling = false;
    var btn = $("pollBtn");
    btn.textContent = "▶ 持续观察";
    btn.classList.remove("running");
  }

  function stopTimer() {
    if (state.timer) { clearInterval(state.timer); state.timer = null; }
  }

  async function tick() {
    if (!state.polling || !state.connectionId || !state.watch.length) return;
    if (state.tickBusy) return; // skip if the previous round is still in flight
    state.tickBusy = true;
    try {
      var nodeIds = state.watch.map(function (i) { return i.nodeId; });
      var res = unwrap(
        await invoke("opcua/readMany", { id: state.connectionId, nodeIds: nodeIds }, 15000)
      );
      applyValues((res && res.values) || []);
      setStatus("观察中 · " + state.watch.length + " 点", "ok");
    } catch (e) {
      stopPolling();
      setStatus("轮询失败", "err");
      toast("轮询失败：" + msg(e) + "（若提示未连接，请先在 DBX 中打开该连接）", true);
    } finally {
      state.tickBusy = false;
    }
  }

  // ----------------------------------------------------------------------
  // Persistence (best effort — sandboxed storage may be unavailable)
  // ----------------------------------------------------------------------
  function saveWatch() {
    if (!state.connectionId) return;
    try {
      var slim = state.watch.map(function (i) {
        return { nodeId: i.nodeId, displayName: i.displayName, browseName: i.browseName };
      });
      localStorage.setItem(WATCH_KEY_PREFIX + state.connectionId, JSON.stringify(slim));
    } catch (e) { /* ignore */ }
  }

  function loadWatch(connId) {
    state.watch = [];
    try {
      var raw = localStorage.getItem(WATCH_KEY_PREFIX + connId);
      if (!raw) return;
      var arr = JSON.parse(raw);
      if (!Array.isArray(arr)) return;
      arr.forEach(function (i) {
        if (!i || !i.nodeId) return;
        if (findItem(i.nodeId)) return;
        state.watch.push({
          nodeId: i.nodeId,
          displayName: i.displayName || i.browseName || i.nodeId,
          browseName: i.browseName || "",
          dataType: "", value: null, status: "", sourceTimestamp: "",
        });
      });
    } catch (e) { /* ignore */ }
  }

  // ----------------------------------------------------------------------
  // Bootstrap — start only after `dbxPlugin.ready` resolves.
  // ----------------------------------------------------------------------
  async function boot() {
    if (!window.dbxPlugin) {
      setStatus("未在 DBX 宿主中运行", "err");
      return;
    }
    await window.dbxPlugin.ready;

    bindDrop();

    $("useBtn").addEventListener("click", function () {
      useConnection($("connId").value.trim() || $("connSelect").value);
    });
    $("connSelect").addEventListener("change", function () {
      if ($("connSelect").value) useConnection($("connSelect").value);
    });
    $("pollBtn").addEventListener("click", function () {
      if (state.polling) stopPolling(); else startPolling();
    });
    $("pollMs").addEventListener("change", function () {
      if (state.polling) startPolling();
    });
    $("clearWatch").addEventListener("click", function () {
      if (!state.watch.length) return;
      state.watch = [];
      saveWatch();
      renderWatch();
      syncWatchButtons();
    });

    var conns = await loadConnections();
    state.connections = conns;
    renderConnections(conns);

    // Prefer the workbench context's connectionId — the host opens this workbench
    // for a specific connection, so it is authoritative.
    var ctx = window.dbxPlugin.context || {};
    var ctxId = ctx.connectionId || null;
    var opcua = conns.filter(connMatches);

    if (ctxId) {
      $("connId").value = ctxId;
      await useConnection(ctxId);
    } else if (opcua.length) {
      await useConnection(opcua[0].id);
    } else {
      // Never fall back to an unrelated connection (that is how a Modbus device
      // used to show up here).
      setStatus("未发现 OPC UA 连接", "err");
      toast("未发现 OPC UA 连接，请在上方选择或填写连接 ID", true);
    }

    if (typeof window.dbxPlugin.onContext === "function") {
      window.dbxPlugin.onContext(function (next) {
        var id = next && next.connectionId;
        if (id && id !== state.connectionId) {
          $("connId").value = id;
          useConnection(id);
        }
      });
    }
  }

  window.addEventListener("DOMContentLoaded", function () {
    boot().catch(function (e) {
      setStatus("初始化失败", "err");
      toast("初始化失败：" + msg(e), true);
    });
  });
})();
