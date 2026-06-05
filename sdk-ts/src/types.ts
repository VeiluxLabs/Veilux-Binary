// Wire types mirroring the Rust kernel's serde representation exactly.

/** A party identifier. Serializes as a bare JSON string (newtype struct). */
export type PartyId = string;

/**
 * Disclosure scope for an event.
 * Rust serde: `Public` -> "Public", `Parties(vec)` -> { "Parties": [...] }.
 */
export type Visibility = "Public" | { Parties: PartyId[] };

/**
 * A command. `payload` is a byte array (Rust `Vec<u8>` -> JSON number array).
 */
export interface Command {
  prism: string;
  submitter: PartyId;
  visibility: Visibility;
  payload: number[];
  nonce: number;
}

/** A signed command ready for submission. */
export interface SignedCommand {
  command: Command;
  public_key: number[];
  signature: number[];
}

// ---- RPC contract types (mirror veilux-rpc::types) ----

export interface NodeInfo {
  network: string;
  protocol: string;
  token: string;
  height: number;
  head_hash: string;
  state_root: string;
  prisms: string[];
}

export interface SubmitResult {
  accepted: boolean;
  command_id: string;
  mempool_len: number;
}

export interface BlockView {
  height: number;
  hash: string;
  parent: string;
  state_root: string;
  events_root: string;
  proposer: string;
  timestamp: number;
  command_count: number;
  event_count: number;
}

export interface StateResult {
  key: string;
  found: boolean;
  value_hex: string;
}

export interface EstimateResult {
  cost: number;
}

export const RPC_METHODS = {
  nodeInfo: "veilux_nodeInfo",
  submit: "veilux_submit",
  blockNumber: "veilux_blockNumber",
  getBlockByNumber: "veilux_getBlockByNumber",
  getState: "veilux_getState",
  estimate: "veilux_estimate",
} as const;
