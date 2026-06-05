/**
 * Explorer API demo: submit activity, then query the chain like a block
 * explorer / indexer would.
 *
 *   veilux serve --addr 127.0.0.1:8650
 *   node examples-dist/explorer.js
 */
import { Client, PartyIdentity, builders, tokenId } from "../dist/index.js";

const rpc = process.env.VEILUX_RPC ?? "http://127.0.0.1:8650";

async function main() {
  const client = new Client(rpc);
  const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));

  // Generate some activity.
  const create = builders.tokenCreate("alice", "Public", 0, "Gold", "GLD", 18, 1_000_000n, true);
  const createRes = await client.submit(alice.sign(create));
  const tid = tokenId("alice", "GLD", "Gold");
  await client.submit(alice.sign(builders.tokenTransfer("alice", "Public", 1, tid, "bob", 250_000n)));

  // --- Explorer queries ---
  const stats = await client.explorerStats();
  console.log("STATS:", JSON.stringify(stats));

  const recent = await client.recentBlocks(5);
  console.log(`RECENT BLOCKS: ${recent.length} (heights ${recent.map((b) => b.height).join(",")})`);

  const loc = await client.searchCommand(createRes.command_id);
  console.log(`SEARCH cmd ${createRes.command_id.slice(0, 12)}.. -> block #${loc.block_height} prism=${loc.prism} events=${loc.events.length}`);

  const tokenEvents = await client.listByPrism("token", 10);
  console.log(`TOKEN EVENTS: ${tokenEvents.length}`);
  for (const e of tokenEvents) {
    console.log("  -", JSON.stringify(e.payload_json));
  }

  const tokens = await client.statePrefix("token/meta/", 20);
  console.log(`TOKENS REGISTERED: ${tokens.total}`);
}

main().catch((e) => {
  console.error("error:", e);
  process.exit(1);
});
