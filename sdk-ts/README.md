# @veilux/sdk

TypeScript/JavaScript SDK for [VEILUX](https://github.com/VeiluxLabs/Veilux-Binary).
Build, sign, and submit commands to a VEILUX node from Node.js or the browser.

Signing (Ed25519) and hashing (BLAKE3) are **byte-compatible with the Rust
node**, so signatures produced here verify on-chain.

## Install

```bash
npm install @veilux/sdk
```

## Quick start

```ts
import { Client, PartyIdentity, builders, hashCommit, toHex } from "@veilux/sdk";

const client = new Client("http://127.0.0.1:8645");

// 32-byte seed (use real entropy in production)
const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));

const info = await client.nodeInfo();
console.log("height", info.height, "prisms", info.prisms);

// Create a token
const create = builders.tokenCreate(
  "alice", "Public", 0, "Gold Coin", "GLD", 18, 1_000_000n, true,
);
const res = await client.submit(alice.sign(create));
console.log("accepted", res.accepted);

// Transfer (token id is derived the same way the node does)
const te = new TextEncoder();
const tokenId = toHex(hashCommit("token/id",
  [te.encode("alice"), te.encode("GLD"), te.encode("Gold Coin")]));
await client.submit(alice.sign(
  builders.tokenTransfer("alice", "Public", 1, tokenId, "bob", 250_000n)));

// Read state
const bal = await client.getState(`token/bal/${tokenId}/bob`);
console.log("bob:", Buffer.from(bal.value_hex, "hex").toString());
```

## API

### `PartyIdentity`
- `PartyIdentity.fromSeed(party, seed: Uint8Array)` — 32-byte seed
- `PartyIdentity.fromPassphrase(party, passphrase)` — dev convenience
- `PartyIdentity.generate(party)` — random
- `id.publicKey(): Uint8Array`
- `id.sign(command): SignedCommand`

### `Client`
- `nodeInfo()`, `blockNumber()`, `blockByNumber(h)`, `getState(key)`
- `estimate(signed)`, `submit(signed)`

### `builders`
- `tokenCreate`, `tokenTransfer`
- `nftCreateCollection`
- `storagePut`
- `contractDeploy`, `contractCall`

### Encoding helpers
- `signingBytes(command)`, `commandId(command)`, `hashCommit(domain, parts)`, `toHex(bytes)`

## Compatibility notes

- `Hash`-typed command fields (token id, contract address) are byte arrays on
  the wire; the builders convert hex for you.
- Token amounts are `u128` and serialized as decimal strings.
- `Visibility` is `"Public"` or `{ Parties: ["alice", ...] }`.

## License

MIT
