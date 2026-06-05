"use strict";

// ---------- Config & RPC ----------
const DEFAULT_RPC = localStorage.getItem("veilux_rpc") || "http://127.0.0.1:8645";
let RPC = DEFAULT_RPC;
let WS = null;
let rpcId = 1;

async function rpc(method, params = {}) {
  const res = await fetch(RPC, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ jsonrpc: "2.0", method, params, id: rpcId++ }),
  });
  const json = await res.json();
  if (json.error) throw new Error(json.error.message);
  return json.result;
}

// ---------- Helpers ----------
const $ = (id) => document.getElementById(id);
const el = (tag, cls, html) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (html !== undefined) e.innerHTML = html;
  return e;
};
const short = (s, n = 10) => (s && s.length > n * 2 ? `${s.slice(0, n)}…${s.slice(-6)}` : s);
const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" }[c]));
const num = (n) => Number(n).toLocaleString("en-US");
function ago(ts) {
  if (!ts) return "—";
  const d = Math.max(0, Math.floor(Date.now() / 1000) - Number(ts));
  if (d < 60) return `${d}s ago`;
  if (d < 3600) return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

// ---------- Status ----------
function setStatus(online) {
  const dot = $("statusDot");
  dot.className = "status-dot " + (online ? "online" : "offline");
  dot.title = online ? "online" : "offline";
}

// ---------- Routing ----------
window.addEventListener("hashchange", route);

const VIEWS = ["view-home", "view-detail", "view-verify", "view-docs"];
function showView(idToShow) {
  for (const v of VIEWS) {
    const e = $(v);
    if (e) e.classList.toggle("hidden", v !== idToShow);
  }
  document.querySelectorAll(".nav-tabs a").forEach((a) => a.classList.remove("active"));
}

function route() {
  const hash = location.hash.slice(1) || "/";
  const [, kind, id] = hash.split("/");
  if (kind === "block") {
    showView("view-detail");
    $("detail").innerHTML = '<div class="empty">Loading…</div>';
    renderBlockDetail(id);
  } else if (kind === "tx") {
    showView("view-detail");
    $("detail").innerHTML = '<div class="empty">Loading…</div>';
    renderTxDetail(id);
  } else if (kind === "contract" || kind === "address") {
    showView("view-detail");
    $("detail").innerHTML = '<div class="empty">Loading…</div>';
    renderContractDetail(id);
  } else if (kind === "verify") {
    showView("view-verify");
    setTab("verify");
  } else if (kind === "docs") {
    showView("view-docs");
    setTab("docs");
    renderDocs();
  } else {
    showView("view-home");
    setTab("home");
  }
}

function setTab(name) {
  const a = document.querySelector(`.nav-tabs a[data-tab="${name}"]`);
  if (a) a.classList.add("active");
}

// ---------- Home: stats ----------
let lastStatValues = {};
async function loadStats() {
  try {
    const s = await rpc("explorer_stats");
    setStatus(true);
    const tiles = [
      { key: "height", label: "Block Height", value: s.height, fmt: (v) => `#${num(v)}` },
      { key: "txns", label: "Total Transactions", value: s.total_commands, fmt: num },
      { key: "events", label: "Total Events", value: s.total_events, fmt: num },
      { key: "state", label: "State Entries", value: s.state_entries, fmt: num },
    ];
    if (!$("statRow").dataset.ready) {
      $("statRow").innerHTML = "";
      for (const t of tiles) {
        const c = el("div", "stat-card");
        c.innerHTML = `<div class="label">${t.label}</div><div class="value" id="stat-${t.key}">${t.fmt(t.value)}</div>`;
        $("statRow").appendChild(c);
      }
      $("statRow").dataset.ready = "1";
    } else {
      for (const t of tiles) {
        const elv = $(`stat-${t.key}`);
        if (elv) {
          const changed = lastStatValues[t.key] !== t.value;
          elv.textContent = t.fmt(t.value);
          if (changed && lastStatValues[t.key] !== undefined) {
            elv.classList.remove("bump");
            void elv.offsetWidth; // reflow to restart animation
            elv.classList.add("bump");
          }
        }
      }
    }
    for (const t of tiles) lastStatValues[t.key] = t.value;

    const sel = $("prismFilter");
    const current = sel.value;
    const prisms = Object.keys(s.events_by_prism || {});
    sel.innerHTML = `<option value="">All prisms</option>` +
      prisms.map((p) => `<option value="${esc(p)}">${esc(p)} (${s.events_by_prism[p]})</option>`).join("");
    if (current) sel.value = current;
  } catch (e) {
    setStatus(false);
    if (!$("statRow").dataset.ready) {
      $("statRow").innerHTML = `<div class="stat-card"><div class="label">Connection</div><div class="value"><small>offline — check endpoint</small></div></div>`;
    }
  }
}

// ---------- Home: latest blocks ----------
let lastTopBlock = -1;
async function loadBlocks() {
  try {
    const blocks = await rpc("explorer_recentBlocks", { limit: 12 });
    const box = $("latestBlocks");
    if (!blocks.length) { box.innerHTML = '<div class="empty">No blocks yet</div>'; return; }
    const newTop = blocks[0].height;
    box.innerHTML = "";
    for (const b of blocks) {
      const row = el("div", "row");
      // Animate rows that are newer than what we last rendered.
      if (lastTopBlock >= 0 && b.height > lastTopBlock) row.classList.add("new");
      row.innerHTML = `
        <div class="row-icon">Bk</div>
        <div class="row-main">
          <div class="row-title"><a href="#/block/${b.height}">Block #${num(b.height)}</a></div>
          <div class="row-sub">${ago(b.timestamp)} · proposer ${esc(b.proposer)}</div>
        </div>
        <div class="row-right">
          <div><span class="badge">${b.command_count} txn</span></div>
          <div class="row-sub hashlink">${short(b.hash, 8)}</div>
        </div>`;
      box.appendChild(row);
    }
    lastTopBlock = newTop;
  } catch (e) {
    $("latestBlocks").innerHTML = `<div class="empty">${esc(e.message)}</div>`;
  }
}

// ---------- Home: latest transactions (events) ----------
async function loadTxns() {
  const prism = $("prismFilter").value;
  const box = $("latestTxns");
  try {
    let events = [];
    if (prism) {
      events = await rpc("explorer_listByPrism", { prism, limit: 15 });
    } else {
      // "All" view: merge recent events across every active prism.
      const s = await rpc("explorer_stats");
      for (const p of Object.keys(s.events_by_prism || {})) {
        const evs = await rpc("explorer_listByPrism", { prism: p, limit: 6 });
        events.push(...evs);
      }
      events.sort((a, b) => b.block_height - a.block_height);
      events = events.slice(0, 15);
    }
    if (!events.length) { box.innerHTML = '<div class="empty">No transactions yet</div>'; return; }
    box.innerHTML = "";
    for (const ev of events) {
      const kind = ev.payload_json && ev.payload_json.kind ? ev.payload_json.kind : ev.prism;
      const priv = ev.visibility !== "public";
      const row = el("div", "row");
      row.innerHTML = `
        <div class="row-icon">Tx</div>
        <div class="row-main">
          <div class="row-title"><a href="#/tx/${ev.source_command}">${esc(kind)}</a></div>
          <div class="row-sub">block <a href="#/block/${ev.block_height}">#${num(ev.block_height)}</a> · <span class="badge prism">${esc(ev.prism)}</span></div>
        </div>
        <div class="row-right">
          ${priv ? '<span class="badge private">private</span>' : '<span class="badge">public</span>'}
          <div class="row-sub hashlink">${short(ev.source_command, 8)}</div>
        </div>`;
      box.appendChild(row);
    }
  } catch (e) {
    box.innerHTML = `<div class="empty">${esc(e.message)}</div>`;
  }
}

// ---------- Home: state browser ----------
async function loadState() {
  const prefix = $("statePrefix").value.trim();
  const box = $("stateList");
  box.innerHTML = '<div class="empty">Loading…</div>';
  try {
    const res = await rpc("explorer_statePrefix", { prefix, limit: 50 });
    if (!res.entries.length) { box.innerHTML = `<div class="empty">No entries under "${esc(prefix)}"</div>`; return; }
    box.innerHTML = "";
    const head = el("div", "row");
    head.innerHTML = `<div class="row-main muted">${res.total} entr${res.total === 1 ? "y" : "ies"} under <code>${esc(prefix)}</code></div>`;
    box.appendChild(head);
    for (const e of res.entries) {
      const decoded = hexToMaybeString(e.value_hex);
      const row = el("div", "row");
      row.innerHTML = `
        <div class="row-main">
          <div class="row-title">${esc(e.key)}</div>
          <div class="row-sub ellip">${esc(decoded)}</div>
        </div>`;
      box.appendChild(row);
    }
  } catch (e) {
    box.innerHTML = `<div class="empty">${esc(e.message)}</div>`;
  }
}

function hexToMaybeString(hex) {
  try {
    const bytes = hex.match(/.{1,2}/g) || [];
    const str = bytes.map((b) => String.fromCharCode(parseInt(b, 16))).join("");
    // Printable ASCII heuristic.
    if (/^[\x20-\x7e]*$/.test(str)) return str.length > 120 ? str.slice(0, 120) + "…" : str;
  } catch (_) {}
  return "0x" + hex;
}

// ---------- Detail: block ----------
async function renderBlockDetail(idOrHash) {
  try {
    let block;
    if (/^\d+$/.test(idOrHash)) {
      block = await rpc("veilux_getBlockByNumber", { height: Number(idOrHash) });
    } else {
      block = await rpc("explorer_blockByHash", { hash: idOrHash });
    }
    const kv = [
      ["Height", `#${num(block.height)}`],
      ["Hash", block.hash],
      ["Parent", `<a href="#/block/${block.parent}">${block.parent}</a>`],
      ["State Root", block.state_root],
      ["Events Root", block.events_root],
      ["Proposer", esc(block.proposer)],
      ["Timestamp", `${block.timestamp} (${ago(block.timestamp)})`],
      ["Transactions", num(block.command_count)],
      ["Events", num(block.event_count)],
    ];
    $("detail").innerHTML = `
      <div class="detail-card">
        <h2>Block #${num(block.height)}</h2>
        <div class="kv">${kv.map(([k, v]) => `<div class="k">${k}</div><div class="v">${v}</div>`).join("")}</div>
      </div>`;
  } catch (e) {
    $("detail").innerHTML = `<div class="detail-card"><h2>Not found</h2><div class="empty">${esc(e.message)}</div></div>`;
  }
}

// ---------- Detail: transaction (command) ----------
async function renderTxDetail(commandId) {
  try {
    const loc = await rpc("explorer_searchCommand", { command_id: commandId });
    if (!loc.found) {
      $("detail").innerHTML = `<div class="detail-card"><h2>Transaction not found</h2><div class="empty">${esc(commandId)}</div></div>`;
      return;
    }
    const kv = [
      ["Command ID", loc.command_id],
      ["Status", '<span class="badge">Success</span>'],
      ["Block", `<a href="#/block/${loc.block_height}">#${num(loc.block_height)}</a>`],
      ["Block Hash", loc.block_hash],
      ["Prism", `<span class="badge prism">${esc(loc.prism)}</span>`],
      ["Submitter", esc(loc.submitter)],
      ["Events", num(loc.events.length)],
    ];
    const eventsHtml = loc.events.map((ev) => {
      const body = ev.payload_json
        ? `<pre class="json">${esc(JSON.stringify(ev.payload_json, null, 2))}</pre>`
        : `<pre class="json">0x${esc(ev.payload_hex || "")}</pre>`;
      return `
        <div class="detail-card">
          <h2>Event · <span class="badge prism">${esc(ev.prism)}</span> ${ev.visibility !== "public" ? '<span class="badge private">private</span>' : ""}</h2>
          ${body}
        </div>`;
    }).join("");
    $("detail").innerHTML = `
      <div class="detail-card">
        <h2>Transaction</h2>
        <div class="kv">${kv.map(([k, v]) => `<div class="k">${k}</div><div class="v">${v}</div>`).join("")}</div>
      </div>
      ${eventsHtml}`;
  } catch (e) {
    $("detail").innerHTML = `<div class="detail-card"><h2>Error</h2><div class="empty">${esc(e.message)}</div></div>`;
  }
}

// ---------- Universal search ----------
async function doSearch(q) {
  q = q.trim();
  if (!q) return;
  if (/^\d+$/.test(q)) { location.hash = `#/block/${q}`; return; }
  // 0x + 64 hex: could be a block hash, command id, or contract address.
  if (/^0x[0-9a-fA-F]{64}$/.test(q)) {
    const loc = await rpc("explorer_searchCommand", { command_id: q }).catch(() => null);
    if (loc && loc.found) { location.hash = `#/tx/${q}`; return; }
    const code = await rpc("contract_getCode", { address: q }).catch(() => null);
    if (code && code.found) { location.hash = `#/contract/${q}`; return; }
    location.hash = `#/block/${q}`;
    return;
  }
  alert("Enter a block height, a 0x… hash, a command id, or a contract address.");
}

// ---------- Live updates via WebSocket ----------
function connectWs() {
  if (WS) { try { WS.close(); } catch (_) {} WS = null; }
  // WS port = RPC port + 1 by convention.
  try {
    const u = new URL(RPC);
    const wsPort = (parseInt(u.port || "8645", 10) + 1).toString();
    const wsUrl = `ws://${u.hostname}:${wsPort}`;
    WS = new WebSocket(wsUrl);
    WS.onmessage = (ev) => {
      try {
        const msg = JSON.parse(ev.data);
        if (msg.type === "block") refreshHome();
      } catch (_) {}
    };
    WS.onerror = () => {};
  } catch (_) {}
}

// ---------- Orchestration ----------
let refreshTimer = null;
function refreshHome() {
  loadStats();
  loadBlocks();
  loadTxns();
}

function connect() {
  RPC = $("endpoint").value.trim() || DEFAULT_RPC;
  localStorage.setItem("veilux_rpc", RPC);
  refreshHome();
  loadState();
  connectWs();
}

function init() {
  $("endpoint").value = RPC;
  $("connectBtn").onclick = connect;
  $("navSearchBtn").onclick = () => doSearch($("navSearch").value);
  $("navSearch").addEventListener("keydown", (e) => { if (e.key === "Enter") doSearch($("navSearch").value); });
  $("prismFilter").onchange = loadTxns;
  $("stateBtn").onclick = loadState;
  $("statePrefix").addEventListener("keydown", (e) => { if (e.key === "Enter") loadState(); });
  initVerify();

  route();
  refreshHome();
  loadState();
  connectWs();

  refreshTimer = setInterval(() => {
    if (!location.hash || location.hash === "#/") refreshHome();
  }, 8000);
}

document.addEventListener("DOMContentLoaded", init);

// ---------- Detail: contract ----------
async function renderContractDetail(address) {
  try {
    const code = await rpc("contract_getCode", { address });
    if (!code.found) {
      $("detail").innerHTML = `<div class="detail-card"><h2>Contract not found</h2><div class="empty">${esc(address)}</div></div>`;
      return;
    }
    const verBadge = code.verified
      ? `<span class="verified-tag"><svg viewBox="0 0 24 24" width="16" height="16"><path fill="currentColor" d="m9 16.2-3.5-3.5L4 14.2 9 19l11-11-1.5-1.5z"/></svg>Verified</span>`
      : `<a href="#/verify" class="badge">Unverified — verify ↗</a>`;
    const kv = [
      ["Address", code.address],
      ["Status", verBadge],
      ["Deployer", esc(code.deployer || "—")],
      ["Code Size", `${num(code.code_size)} bytes`],
      ["Code Hash", code.code_hash],
    ];
    let verifiedBlock = "";
    if (code.verified) {
      const v = await rpc("contract_getVerification", { address }).catch(() => null);
      if (v && v.found && v.record) {
        const r = v.record;
        verifiedBlock = `
          <div class="detail-card">
            <h2>Verified Source · ${esc(r.name)}</h2>
            <div class="kv">
              <div class="k">Compiler</div><div class="v">${esc(r.compiler)}</div>
              <div class="k">Verified at</div><div class="v">block #${num(r.verified_at_height)}</div>
            </div>
            <pre class="json">${esc(r.source)}</pre>
            ${r.abi ? `<pre class="json">${esc(r.abi)}</pre>` : ""}
          </div>`;
      }
    }
    $("detail").innerHTML = `
      <div class="detail-card">
        <h2>Contract</h2>
        <div class="kv">${kv.map(([k, v]) => `<div class="k">${k}</div><div class="v">${v}</div>`).join("")}</div>
      </div>
      <div class="detail-card">
        <h2>Deployed Bytecode</h2>
        <pre class="json">0x${esc(code.bytecode_hex)}</pre>
      </div>
      ${verifiedBlock}`;
  } catch (e) {
    $("detail").innerHTML = `<div class="detail-card"><h2>Error</h2><div class="empty">${esc(e.message)}</div></div>`;
  }
}

// ---------- Verify page ----------
function initVerify() {
  $("vLoadBtn").onclick = async () => {
    const addr = $("vAddress").value.trim();
    if (!addr) return;
    try {
      const code = await rpc("contract_getCode", { address: addr });
      if (code.found) {
        $("vBytecode").value = code.bytecode_hex;
        flashResult($("vResult"), true, `Loaded ${code.code_size} bytes of on-chain bytecode.`);
      } else {
        flashResult($("vResult"), false, "Contract not found at that address.");
      }
    } catch (e) {
      flashResult($("vResult"), false, e.message);
    }
  };
  $("vSubmitBtn").onclick = async () => {
    const req = {
      address: $("vAddress").value.trim(),
      name: $("vName").value.trim() || "Contract",
      source: $("vSource").value,
      bytecode_hex: $("vBytecode").value.trim(),
      compiler: $("vCompiler").value.trim() || "photonvm-asm",
      abi: $("vAbi").value.trim(),
    };
    if (!req.address || !req.bytecode_hex) {
      flashResult($("vResult"), false, "Address and bytecode are required.");
      return;
    }
    try {
      const res = await rpc("contract_verify", req);
      flashResult($("vResult"), res.verified, res.message + (res.verified ? ` (hash ${short(res.code_hash, 8)})` : ""));
      if (res.verified) {
        setTimeout(() => { location.hash = `#/contract/${req.address}`; }, 1200);
      }
    } catch (e) {
      flashResult($("vResult"), false, e.message);
    }
  };
}

function flashResult(box, ok, msg) {
  box.innerHTML = `<div class="verify-result ${ok ? "ok" : "bad"}">${ok ? "✓" : "✗"} ${esc(msg)}</div>`;
}

// ---------- Docs page ----------
function renderDocs() {
  const c = $("docsContent");
  if (c.dataset.rendered) return;
  c.dataset.rendered = "1";
  c.innerHTML = `
    <h1>VEILUX Explorer & RPC API</h1>
    <p class="muted">All endpoints are JSON-RPC 2.0 over HTTP POST. The node also
    serves a WebSocket endpoint (RPC port + 1) that streams new blocks.</p>

    <h2>Core</h2>
    <table>
      <tr><th>Method</th><th>Params</th><th>Returns</th></tr>
      <tr><td><code>veilux_nodeInfo</code></td><td>{}</td><td>network, height, prisms</td></tr>
      <tr><td><code>veilux_blockNumber</code></td><td>{}</td><td>current height</td></tr>
      <tr><td><code>veilux_getBlockByNumber</code></td><td>{ height }</td><td>block</td></tr>
      <tr><td><code>veilux_getState</code></td><td>{ key }</td><td>{ found, value_hex }</td></tr>
      <tr><td><code>veilux_estimate</code></td><td>{ command }</td><td>{ cost }</td></tr>
      <tr><td><code>veilux_submit</code></td><td>{ command }</td><td>{ accepted, command_id }</td></tr>
    </table>

    <h2>Explorer</h2>
    <table>
      <tr><th>Method</th><th>Params</th><th>Returns</th></tr>
      <tr><td><code>explorer_stats</code></td><td>{}</td><td>chain totals + per-prism counts</td></tr>
      <tr><td><code>explorer_recentBlocks</code></td><td>{ limit }</td><td>newest blocks first</td></tr>
      <tr><td><code>explorer_blockByHash</code></td><td>{ hash }</td><td>block</td></tr>
      <tr><td><code>explorer_searchCommand</code></td><td>{ command_id }</td><td>block + events</td></tr>
      <tr><td><code>explorer_listByPrism</code></td><td>{ prism, limit }</td><td>events</td></tr>
      <tr><td><code>explorer_statePrefix</code></td><td>{ prefix, limit }</td><td>state entries</td></tr>
    </table>

    <h2>Contract Verification</h2>
    <table>
      <tr><th>Method</th><th>Params</th><th>Returns</th></tr>
      <tr><td><code>contract_getCode</code></td><td>{ address }</td><td>bytecode + verified flag</td></tr>
      <tr><td><code>contract_verify</code></td><td>{ address, name, source, bytecode_hex, compiler, abi }</td><td>{ verified, message }</td></tr>
      <tr><td><code>contract_getVerification</code></td><td>{ address }</td><td>{ found, record }</td></tr>
    </table>

    <h2>Example: get chain stats</h2>
    <pre>curl -s http://127.0.0.1:8645 \\
  -d '{"jsonrpc":"2.0","method":"explorer_stats","params":{},"id":1}'</pre>

    <h2>Example: verify a contract (JS SDK)</h2>
    <pre>import { Client } from "@veilux/sdk";
const client = new Client("http://127.0.0.1:8645");
const code = await client.contractGetCode("0x…");
const res = await client.contractVerify({
  address: "0x…", name: "Adder", compiler: "photonvm-asm 1.0",
  source: "; assembly…", bytecode_hex: code.bytecode_hex, abi: "",
});
console.log(res.verified);</pre>

    <h2>WebSocket subscriptions</h2>
    <pre>const ws = new WebSocket("ws://127.0.0.1:8646");
ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  if (msg.type === "block") console.log("new block", msg.height);
};</pre>

    <p class="muted">Full SDK docs: <a href="https://github.com/VeiluxLabs/Veilux-Binary/blob/main/docs/rpc-sdk.md" target="_blank" rel="noopener">docs/rpc-sdk.md</a></p>
  `;
}
