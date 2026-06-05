/**
 * End-to-end TypeScript example against a `veilux serve` dev node.
 *
 *   veilux serve --addr 127.0.0.1:8645
 *   node --loader ts-node/esm examples/quickstart.ts
 *
 * Or after `npm run build`, compile this file and run with node.
 */
import { Client, PartyIdentity, builders, hashCommit, toHex } from "../dist/index.js";

const endpoint = process.env.VEILUX_RPC ?? "http://127.0.0.1:8645";

async function main() {
  const client = new Client(endpoint);

  // Deterministic identity: 32-byte seed of all 1s (matches Rust [1u8;32]).
  const seed = new Uint8Array(32).fill(1);
  const alice = PartyIdentity.fromSeed("alice", seed);

  const info = await client.nodeInfo();
  console.log(`node: network=${info.network} height=${info.height} prisms=${info.prisms.join(",")}`);

  // 1) Create a token.
  const create = builders.tokenCreate("alice", "Public", 0, "Gold Coin", "GLD", 18, 1_000_000n, true);
  const est = await client.estimate(alice.sign(create));
  console.log(`estimated cost: ${est.cost} LUX`);

  const res = await client.submit(alice.sign(create));
  console.log(`token create accepted=${res.accepted} id=${res.command_id}`);

  // 2) Compute the token id the same way the node does, then transfer.
  const te = new TextEncoder();
  const tokenId = toHex(
    hashCommit("token/id", [te.encode("alice"), te.encode("GLD"), te.encode("Gold Coin")]),
  );
  const transfer = builders.tokenTransfer("alice", "Public", 1, tokenId, "bob", 250_000n);
  const res2 = await client.submit(alice.sign(transfer));
  console.log(`transfer accepted=${res2.accepted}`);

  // 3) Read state back.
  const height = await client.blockNumber();
  const block = await client.blockByNumber(height);
  console.log(`head block #${block.height} hash=${block.hash} commands=${block.command_count}`);

  const balKey = `token/bal/${tokenId}/bob`;
  const bal = await client.getState(balKey);
  if (bal.found) {
    const decimal = Buffer.from(bal.value_hex, "hex").toString("utf8");
    console.log(`bob's GLD balance: ${decimal}`);
  } else {
    console.log(`bob's GLD balance: 0 (not found)`);
  }
}

main().catch((e) => {
  console.error("error:", e);
  process.exit(1);
});
