use veilux_evm::{address_from_secret, sign_legacy_tx};

fn parse_to(s: &str) -> Option<[u8; 20]> {
    let s = s.trim_start_matches("0x");
    if s.is_empty() || s == "null" {
        return None;
    }
    let bytes = hex::decode(s).ok()?;
    if bytes.len() != 20 {
        return None;
    }
    let mut a = [0u8; 20];
    a.copy_from_slice(&bytes);
    Some(a)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let sk_hex = args
        .get(1)
        .expect("usage: sign_tx <sk_hex> <to|null> <value> <nonce> <chain_id> [data_hex]");
    let to_arg = args.get(2).expect("to");
    let value: u128 = args.get(3).expect("value").parse().unwrap();
    let nonce: u64 = args.get(4).expect("nonce").parse().unwrap();
    let chain_id: u64 = args.get(5).expect("chain_id").parse().unwrap();
    let data: Vec<u8> = args
        .get(6)
        .map(|d| hex::decode(d.trim_start_matches("0x")).unwrap())
        .unwrap_or_default();

    let mut sk = [0u8; 32];
    sk.copy_from_slice(&hex::decode(sk_hex.trim_start_matches("0x")).unwrap());
    let to = parse_to(to_arg);

    let from = address_from_secret(&sk).unwrap();
    let raw = sign_legacy_tx(
        &sk,
        nonce,
        1_000_000_000,
        3_000_000,
        to,
        value,
        &data,
        chain_id,
    )
    .unwrap();

    println!("from=0x{}", hex::encode(from));
    println!("raw=0x{}", hex::encode(raw));
}
