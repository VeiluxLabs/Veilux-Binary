//! End-to-end SDK example: connect to a `veilux serve` dev node, create a
//! token, transfer it, and read the chain back.
//!
//! Run a node first:  veilux serve --addr 127.0.0.1:8645
//! Then:              cargo run -p veilux-sdk --example quickstart

use veilux_sdk::{builders, Client, PartyIdentity, Visibility};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let endpoint =
        std::env::var("VEILUX_RPC").unwrap_or_else(|_| "http://127.0.0.1:8645".to_string());
    let client = Client::new(&endpoint);

    let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
    println!("connected to {endpoint}");

    let info = client.node_info()?;
    println!(
        "node: network={} height={} prisms={:?}",
        info.network, info.height, info.prisms
    );

    // 1) Create a fungible token.
    let create = builders::token_create(
        alice.party().clone(),
        Visibility::Public,
        0,
        "Gold Coin",
        "GLD",
        18,
        1_000_000,
        true,
    );
    let est = client.estimate(&alice.sign(create.clone()))?;
    println!("estimated cost: {} LUX", est.cost);

    let res = client.submit(&alice.sign(create))?;
    println!(
        "token create accepted={} id={}",
        res.accepted, res.command_id
    );

    // 2) Transfer some to bob (token id is derived deterministically).
    let token_id = veilux_sdk::Hash::commit("token/id", &[b"alice", b"GLD", b"Gold Coin"]);
    let transfer = builders::token_transfer(
        alice.party().clone(),
        Visibility::Public,
        1,
        token_id,
        veilux_sdk::PartyId::new("bob"),
        250_000,
    );
    let res = client.submit(&alice.sign(transfer))?;
    println!("transfer accepted={}", res.accepted);

    // 3) Read the chain back.
    let height = client.block_number()?;
    println!("chain height now: {height}");
    let block = client.block_by_number(height)?;
    println!(
        "head block #{} hash={} commands={}",
        block.height, block.hash, block.command_count
    );

    // 4) Read bob's token balance from state (stored as a decimal string).
    let bal_key = format!("token/bal/{}/bob", token_id.to_hex());
    let bal = client.get_state(&bal_key)?;
    if bal.found {
        let decimal =
            String::from_utf8(hex::decode(&bal.value_hex).unwrap_or_default()).unwrap_or_default();
        println!("bob's GLD balance: {decimal}");
    } else {
        println!("bob's GLD balance: 0 (key {bal_key} not found)");
    }

    Ok(())
}
