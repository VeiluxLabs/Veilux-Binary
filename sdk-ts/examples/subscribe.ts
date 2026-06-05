/**
 * WebSocket subscription example. Subscribes to new blocks, then submits a
 * command and watches the block notification arrive in real time.
 *
 *   veilux serve --addr 127.0.0.1:8645   (also opens ws on 8646)
 *   node examples-dist/subscribe.js
 */
import { Client, PartyIdentity, builders, subscribeBlocks } from "../dist/index.js";

const rpc = process.env.VEILUX_RPC ?? "http://127.0.0.1:8645";
const ws = process.env.VEILUX_WS ?? "ws://127.0.0.1:8646";

async function main() {
  const client = new Client(rpc);
  const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));

  let received = 0;
  const sub = subscribeBlocks(ws, {
    onOpen: () => console.log("subscribed to", ws),
    onBlock: (b) => {
      received++;
      console.log(`block #${b.height} hash=${b.hash.slice(0, 18)}... cmds=${b.command_count}`);
    },
    onError: (e) => console.error("ws error", e),
  });

  // Give the socket a moment to connect.
  await new Promise((r) => setTimeout(r, 500));

  // Submit two commands; each mints a block the subscriber should see.
  await client.submit(alice.sign(
    builders.tokenCreate("alice", "Public", 0, "Gold", "GLD", 18, 1_000_000n, true)));
  await client.submit(alice.sign(
    builders.storagePut("alice", "Public", 1, "note", Array.from(new TextEncoder().encode("hello")))));

  // Wait for notifications to flow.
  await new Promise((r) => setTimeout(r, 1500));
  console.log(`received ${received} block notification(s)`);
  sub.close();
  process.exit(received >= 2 ? 0 : 1);
}

main().catch((e) => {
  console.error("error:", e);
  process.exit(1);
});
