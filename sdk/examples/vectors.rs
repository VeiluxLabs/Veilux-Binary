//! Prints cross-language test vectors so the TypeScript SDK can assert
//! byte-for-byte compatibility (signing bytes, command id, signature).

use veilux_sdk::{builders, Hash, PartyIdentity, Visibility};
use veilux_kernel::Command;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn dump(label: &str, id: &PartyIdentity, cmd: Command) {
    let signing = cmd.signing_bytes();
    let cid = cmd.id();
    let signed = id.sign(cmd);
    println!("== {label} ==");
    println!("signing_bytes: {}", hex(&signing));
    println!("command_id:    {}", cid.to_hex());
    println!("public_key:    {}", hex(&signed.public_key));
    println!("signature:     {}", hex(&signed.signature));
}

fn main() {
    let alice = PartyIdentity::from_seed("alice", &[1u8; 32]);
    println!("public_key(alice): {}", hex(&alice.public_key()));

    // token create
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
    dump("token_create", &alice, create);

    // token transfer (token id derived)
    let token_id = Hash::commit("token/id", &[b"alice", b"GLD", b"Gold Coin"]);
    let transfer = builders::token_transfer(
        alice.party().clone(),
        Visibility::Parties(vec![veilux_sdk::PartyId::new("alice"), veilux_sdk::PartyId::new("bob")]),
        1,
        token_id,
        veilux_sdk::PartyId::new("bob"),
        250_000,
    );
    dump("token_transfer_private", &alice, transfer);

    println!("token_id: {}", token_id.to_hex());
}
