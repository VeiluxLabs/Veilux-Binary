import { blake3 } from "@noble/hashes/blake3";
import type { Command, Visibility } from "./types.js";

const textEncoder = new TextEncoder();

/** Concatenate byte arrays. */
function concat(...parts: Uint8Array[]): Uint8Array {
  let len = 0;
  for (const p of parts) len += p.length;
  const out = new Uint8Array(len);
  let off = 0;
  for (const p of parts) {
    out.set(p, off);
    off += p.length;
  }
  return out;
}

function u64le(n: number): Uint8Array {
  const buf = new Uint8Array(8);
  const view = new DataView(buf.buffer);
  view.setBigUint64(0, BigInt(n), true);
  return buf;
}

/**
 * Serialize `Visibility` exactly like serde_json on the Rust side, so the
 * signed bytes match byte-for-byte:
 *   Public        -> "Public"   (a quoted JSON string)
 *   Parties([...]) -> {"Parties":["alice","bob"]}
 */
export function visibilityJson(v: Visibility): Uint8Array {
  if (v === "Public") {
    return textEncoder.encode('"Public"');
  }
  // serde_json emits compact JSON with no spaces.
  const inner = v.Parties.map((p) => JSON.stringify(p)).join(",");
  return textEncoder.encode(`{"Parties":[${inner}]}`);
}

/**
 * Reproduce `Command::signing_bytes` from the Rust kernel:
 *   b"veilux/command/v1" 0xff prism 0xff submitter 0xff nonce_le vis 0xff payload
 */
export function signingBytes(cmd: Command): Uint8Array {
  const sep = new Uint8Array([0xff]);
  return concat(
    textEncoder.encode("veilux/command/v1"),
    sep,
    textEncoder.encode(cmd.prism),
    sep,
    textEncoder.encode(cmd.submitter),
    sep,
    u64le(cmd.nonce),
    visibilityJson(cmd.visibility),
    sep,
    Uint8Array.from(cmd.payload),
  );
}

/**
 * Reproduce `Hash::commit(domain, parts)`:
 *   blake3(domain || 0xff || for each part: len_le_u64 || part)
 */
export function hashCommit(domain: string, parts: Uint8Array[]): Uint8Array {
  const chunks: Uint8Array[] = [textEncoder.encode(domain), new Uint8Array([0xff])];
  for (const p of parts) {
    chunks.push(u64le(p.length));
    chunks.push(p);
  }
  return blake3(concat(...chunks));
}

/** Reproduce `Command::id` -> 32-byte BLAKE3 hash. */
export function commandId(cmd: Command): Uint8Array {
  return hashCommit("command", [
    textEncoder.encode(cmd.prism),
    textEncoder.encode(cmd.submitter),
    u64le(cmd.nonce),
    Uint8Array.from(cmd.payload),
  ]);
}

export function toHex(bytes: Uint8Array): string {
  return "0x" + Array.from(bytes).map((b) => b.toString(16).padStart(2, "0")).join("");
}

export { concat, u64le };
