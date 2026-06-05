# VEILUX RPC & SDK

VEILUX exposes a JSON-RPC API so applications can talk to a node, and ships a
Rust SDK that wraps it with identity, command builders, and a typed client.

---

## 1. Running a dev RPC node

```bash
veilux serve --addr 127.0.0.1:8645 --datadir ./veilux-dev-data
```

This starts a persistent single node with all Prisms installed and a JSON-RPC
endpoint. It behaves like a local dev chain: each accepted command is applied
and a block is produced immediately, so clients get fast, deterministic
feedback.

---

## 2. JSON-RPC API

Transport: HTTP POST, JSON-RPC 2.0, `Content-Type: application/json`.

| Method | Params | Result |
|--------|--------|--------|
| `veilux_nodeInfo` | `{}` | network, protocol, token, height, head hash, state root, prisms |
| `veilux_blockNumber` | `{}` | current height (u64) |
| `veilux_getBlockByNumber` | `{ "height": u64 }` | block view (hash, roots, proposer, counts) |
| `veilux_getState` | `{ "key": string }` | `{ found, value_hex }` |
| `veilux_estimate` | `{ "command": SignedCommand }` | `{ cost }` |
| `veilux_submit` | `{ "command": SignedCommand }` | `{ accepted, command_id, mempool_len }` |

### Example

```bash
curl -s http://127.0.0.1:8645 \
  -d '{"jsonrpc":"2.0","method":"veilux_nodeInfo","params":{},"id":1}'
```

```json
{"jsonrpc":"2.0","result":{"network":"veilux-dev","protocol":"photon/1.0",
"token":"LUX","height":0,"head_hash":"0x..","state_root":"0x..",
"prisms":["ai","storage","token","nft","contract"]},"id":1}
```

### Error codes

Standard JSON-RPC codes, plus `-32000` (`COMMAND_REJECTED`) when the node
rejects a command (bad signature, replayed nonce, unknown prism, etc.).

---

## 3. Rust SDK (`veilux-sdk`)

Add the dependency (path or git), then:

```rust
use veilux_sdk::{builders, Client, PartyIdentity, Visibility};

let client = Client::new("http://127.0.0.1:8645");
let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);

// Query
let info = client.node_info()?;
println!("height = {}", info.height);

// Build -> sign -> submit
let cmd = builders::token_create(
    alice.party().clone(), Visibility::Public, 0,
    "Gold", "GLD", 18, 1_000_000, true,
);
let est = client.estimate(&alice.sign(cmd.clone()))?;
let res = client.submit(&alice.sign(cmd))?;
println!("accepted={} cost={}", res.accepted, est.cost);

// Read state back
let bal = client.get_state("token/bal/<id>/bob")?;
```

### What the SDK provides

- **`PartyIdentity`** — Ed25519 keypair, `sign(command)`.
- **`Client`** — typed methods for every RPC call (`node_info`, `block_number`,
  `block_by_number`, `get_state`, `estimate`, `submit`).
- **`builders`** — one namespace with command builders for all Prisms:
  - `token_create`, `token_transfer`
  - `nft_create_collection`
  - `contract_deploy`, `contract_call`
  - `storage_put`
  - `ai_register`, `ai_infer`

### Runnable example

```bash
# terminal 1
veilux serve --addr 127.0.0.1:8645

# terminal 2
cargo run -p veilux-sdk --example quickstart
```

Output (abridged):

```
node: network=veilux-dev height=0 prisms=["ai","storage","token","nft","contract"]
estimated cost: 5000 LUX
token create accepted=true id=0x844c..
transfer accepted=true
chain height now: 2
bob's GLD balance: "250000"
```

---

## 3b. TypeScript SDK (`@veilux/sdk`)

For web dApps and Node.js apps. Signing and hashing are byte-compatible with
the Rust node, so TS-signed commands verify on-chain.

```ts
import { Client, PartyIdentity, builders, hashCommit, toHex } from "@veilux/sdk";

const client = new Client("http://127.0.0.1:8645");
const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));

const create = builders.tokenCreate("alice", "Public", 0, "Gold", "GLD", 18, 1_000_000n, true);
await client.submit(alice.sign(create));

const te = new TextEncoder();
const tokenId = toHex(hashCommit("token/id",
  [te.encode("alice"), te.encode("GLD"), te.encode("Gold")]));
await client.submit(alice.sign(
  builders.tokenTransfer("alice", "Public", 1, tokenId, "bob", 250_000n)));

const bal = await client.getState(`token/bal/${tokenId}/bob`);
```

Build & run the example:

```bash
cd sdk-ts
npm install && npm run build
npm test                            # cross-language compatibility tests
node examples-dist/quickstart.js    # against a running `veilux serve`
```

Helper utilities (no manual hashing needed):

```ts
import { tokenId, collectionId, contractAddress, stateKeys } from "@veilux/sdk";

const id = tokenId("alice", "GLD", "Gold");        // derive token id
const bal = await client.tokenBalance(id, "bob");  // -> bigint
await client.waitForHeight(10);                    // poll until height
```

Cross-language compatibility notes:
- `Hash` command fields (token id, contract address) are byte arrays on the
  wire — the builders convert hex automatically.
- Token amounts (`u128`) are decimal strings.
- `Visibility` is `"Public"` or `{ Parties: [...] }`.

---

## 4. Design notes

- The server is a featherweight HTTP/1.1 + JSON-RPC implementation on raw
  `tokio` TCP — no heavy web framework, consistent with the Photon philosophy.
- Wire types live in `veilux-rpc` and are shared by both server and SDK, so the
  contract can never drift between them.
- The dev node mines on submit. A production deployment would expose the same
  RPC surface over the multi-validator consensus path (`veilux validator`)
  rather than instant-mining.

---

## 5. Roadmap

- WebSocket subscriptions (new blocks, events) for reactive apps
- A TypeScript/JavaScript SDK for web dApps
- Auth + rate limiting for public endpoints
- Batched requests
