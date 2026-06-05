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

function route() {
  const hash = location.hash.slice(1) || "/";
  const [, kind, id] = hash.split("/");
  if (kind === "block") {
    showDetail();
    renderBlockDetail(id);
  } else if (kind === "tx") {
    showDetail();
    renderTxDetail(id);
  } else {
    showHome();
  }
}

function showHome() {
  $("view-home").classList.remove("hidden");
  $("view-detail").classList.add("hidden");
}
function showDetail() {
  $("view-home").classList.add("hidden");
  $("view-detail").classList.remove("hidden");
  $("detail").innerHTML = '<div class="empty">Loading…</div>';
}

// ---------- Home: stats ----------
async function loadStats() {
  try {
    const s = await rpc("explorer_stats");
    setStatus(true);
    const tiles = [
      { label: "Block Height", value: `#${num(s.height)}` },
      { label: "Total Transactions", value: num(s.total_commands) },
      { label: "Total Events", value: num(s.total_events) },
      { label: "State Entries", value: num(s.state_entries) },
    ];
    $("statRow").innerHTML = "";
    for (const t of tiles) {
      const c = el("div", "stat-card");
      c.innerHTML = `<div class="label">${t.label}</div><div class="value">${t.value}</div>`;
      $("statRow").appendChild(c);
    }
    // Prism filter options.
    const sel = $("prismFilter");
    const current = sel.value;
    const prisms = Object.keys(s.events_by_prism || {});
    sel.innerHTML = `<option value="">All prisms</option>` +
      prisms.map((p) => `<option value="${esc(p)}">${esc(p)} (${s.events_by_prism[p]})</option>`).join("");
    if (current) sel.value = current;
  } catch (e) {
    setStatus(false);
    $("statRow").innerHTML = `<div class="stat-card"><div class="label">Connection</div><div class="value"><small>offline — check endpoint</small></div></div>`;
  }
}

// ---------- Home: latest blocks ----------
async function loadBlocks() {
  try {
    const blocks = await rpc("explorer_recentBlocks", { limit: 12 });
    const box = $("latestBlocks");
    if (!blocks.length) { box.innerHTML = '<div class="empty">No blocks yet</div>'; return; }
    box.innerHTML = "";
    for (const b of blocks) {
      const row = el("div", "row");
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
  // 0x + 64 hex: could be a block hash or command id. Try command first.
  if (/^0x[0-9a-fA-F]{64}$/.test(q)) {
    const loc = await rpc("explorer_searchCommand", { command_id: q }).catch(() => null);
    if (loc && loc.found) { location.hash = `#/tx/${q}`; return; }
    location.hash = `#/block/${q}`;
    return;
  }
  alert("Enter a block height, a 0x… block hash, or a command id.");
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

  route();
  refreshHome();
  loadState();
  connectWs();

  // Periodic refresh as a fallback to WS.
  refreshTimer = setInterval(() => {
    if (!location.hash || location.hash === "#/") refreshHome();
  }, 8000);
}

document.addEventListener("DOMContentLoaded", init);
