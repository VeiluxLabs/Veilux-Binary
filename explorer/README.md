# VEILUX Explorer

A modern, EVM-explorer-style web UI (Etherscan/Blockscout vibe) for the VEILUX
blockchain. It talks directly to a `veilux serve` JSON-RPC node and updates live
over WebSocket.

It is a **zero-build static site** — plain HTML/CSS/JS, no framework, no
bundler. Open it from any static host or even the local filesystem.

## Features

- **Dashboard** — block height, total transactions, total events, state entries
- **Latest Blocks** & **Latest Transactions** side by side, auto-refreshing
- **Live updates** via WebSocket (new blocks appear instantly)
- **Universal search** — block height, block hash, or command id
- **Block & transaction detail** pages (hash-routed, shareable URLs)
- **Prism filter** for activity, **State Browser** for raw key/value lookups
- Dark, responsive theme

## Run

1. Start a node with RPC + WebSocket:

   ```bash
   veilux serve --addr 127.0.0.1:8645 --ws 127.0.0.1:8646
   ```

2. Serve this folder over HTTP (any static server works):

   ```bash
   # Python
   python -m http.server 3000

   # or Node
   npx serve .
   ```

3. Open <http://127.0.0.1:3000> and set the endpoint to your node
   (default `http://127.0.0.1:8645`). The explorer derives the WebSocket URL as
   RPC port + 1.

## How it works

The UI calls the node's `explorer_*` JSON-RPC methods (`explorer_stats`,
`explorer_recentBlocks`, `explorer_blockByHash`, `explorer_searchCommand`,
`explorer_listByPrism`, `explorer_statePrefix`) plus `veilux_getBlockByNumber`.
The node serves permissive CORS, so the explorer can be hosted on a different
origin than the node.

## Deploy

Because it is static, you can host it on GitHub Pages, Netlify, Vercel, S3, or
IPFS. Just make sure users can reach a VEILUX node's RPC endpoint from their
browser (set it in the endpoint box).
