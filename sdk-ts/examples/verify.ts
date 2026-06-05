/**
 * Contract verification demo: deploy a PhotonVM contract, read its on-chain
 * bytecode, then verify the source against it.
 *
 *   veilux serve --addr 127.0.0.1:8645
 *   node examples-dist/verify.js
 */
import { Client, PartyIdentity, builders, contractAddress } from "../dist/index.js";

const rpc = process.env.VEILUX_RPC ?? "http://127.0.0.1:8645";

// PhotonVM bytecode: PUSH8 111 ; PUSH8 222 ; ADD ; RETURN
function adderBytecode(): number[] {
  const code: number[] = [];
  const push8 = (n: bigint) => {
    code.push(0x60);
    const b = new Uint8Array(8);
    new DataView(b.buffer).setBigUint64(0, n, false);
    code.push(...b);
  };
  push8(111n);
  push8(222n);
  code.push(0x01); // ADD
  code.push(0xf3); // RETURN
  return code;
}

async function main() {
  const client = new Client(rpc);
  const alice = PartyIdentity.fromSeed("alice", new Uint8Array(32).fill(1));

  const code = adderBytecode();
  const deploy = builders.contractDeploy("alice", "Public", 0, code);
  await client.submit(alice.sign(deploy));

  const addr = contractAddress("alice", 0, code);
  console.log("deployed at:", addr);

  const onchain = await client.contractGetCode(addr);
  console.log(`on-chain: found=${onchain.found} size=${onchain.code_size} verified=${onchain.verified}`);

  const res = await client.contractVerify({
    address: addr,
    name: "Adder",
    compiler: "photonvm-asm 1.0",
    source: "; PUSH8 111\n; PUSH8 222\n; ADD\n; RETURN",
    bytecode_hex: onchain.bytecode_hex,
    abi: '{"methods":[{"name":"run","returns":"u64"}]}',
  });
  console.log(`verify: ${res.verified} — ${res.message}`);

  const after = await client.contractGetCode(addr);
  console.log("verified flag now:", after.verified);

  const rec = await client.contractGetVerification(addr);
  console.log("record:", JSON.stringify(rec.record ?? rec));
}

main().catch((e) => {
  console.error("error:", e);
  process.exit(1);
});
