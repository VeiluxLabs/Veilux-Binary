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

/** Real-time notification pushed over WebSocket when a block is committed. */
export interface BlockNotification {
  type: "block";
  height: number;
  hash: string;
  state_root: string;
  command_count: number;
  event_count: number;
  timestamp: number;
}

export const RPC_METHODS = {
  nodeInfo: "veilux_nodeInfo",
  submit: "veilux_submit",
  blockNumber: "veilux_blockNumber",
  getBlockByNumber: "veilux_getBlockByNumber",
  getState: "veilux_getState",
  estimate: "veilux_estimate",
  explorerStats: "explorer_stats",
  explorerRecentBlocks: "explorer_recentBlocks",
  explorerBlockByHash: "explorer_blockByHash",
  explorerSearchCommand: "explorer_searchCommand",
  explorerListByPrism: "explorer_listByPrism",
  explorerStatePrefix: "explorer_statePrefix",
  contractGetCode: "contract_getCode",
  contractVerify: "contract_verify",
  contractGetVerification: "contract_getVerification",
} as const;

// ---- Explorer types ----

export interface ChainStats {
  height: number;
  total_blocks: number;
  total_commands: number;
  total_events: number;
  head_hash: string;
  state_root: string;
  state_entries: number;
  events_by_prism: Record<string, number>;
}

export interface EventView {
  block_height: number;
  prism: string;
  commitment: string;
  source_command: string;
  visibility: string;
  payload_json: unknown | null;
  payload_hex: string | null;
}

export interface CommandLocation {
  found: boolean;
  command_id: string;
  block_height: number | null;
  block_hash: string | null;
  prism: string | null;
  submitter: string | null;
  events: EventView[];
}

export interface StateEntry {
  key: string;
  value_hex: string;
}

export interface StatePrefixResult {
  prefix: string;
  total: number;
  entries: StateEntry[];
}

export interface ContractCode {
  address: string;
  found: boolean;
  deployer: string | null;
  bytecode_hex: string;
  code_size: number;
  code_hash: string;
  verified: boolean;
}

export interface VerifyRequest {
  address: string;
  name: string;
  source: string;
  bytecode_hex: string;
  compiler: string;
  abi?: string;
}

export interface VerifyResult {
  verified: boolean;
  message: string;
  code_hash: string;
}
