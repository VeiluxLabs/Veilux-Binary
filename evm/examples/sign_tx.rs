use veilux_evm::{address_from_secret, sign_legacy_tx};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sk_hex = args
        .get(1)
        .expect("usage: sign_tx <sk_hex> <to_hex> <value> <nonce> <chain_id>");
    let to_hex = args.get(2).expect("to");
    let value: u128 = args.get(3).expect("value").parse().unwrap();
    let nonce: u64 = args.get(4).expect("nonce").parse().unwrap();
    let chain_id: u64 = args.get(5).expect("chain_id").parse().unwrap();

    let mut sk = [0u8; 32];
    sk.copy_from_slice(&hex::decode(sk_hex.trim_start_matches("0x")).unwrap());
    let mut to = [0u8; 20];
    to.copy_from_slice(&hex::decode(to_hex.trim_start_matches("0x")).unwrap());

    let from = address_from_secret(&sk).unwrap();
    let raw = sign_legacy_tx(
        &sk,
        nonce,
        1_000_000_000,
        21_000,
        Some(to),
        value,
        &[],
        chain_id,
    )
    .unwrap();

    println!("from=0x{}", hex::encode(from));
    println!("raw=0x{}", hex::encode(raw));
}
