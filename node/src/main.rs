mod driver;
mod node;
mod rpc_service;
mod validator_loop;
mod viewsync;

use anyhow::Result;
use tracing::info;

use prism_ai::{infer_command, register_command, AiEvent, AiPrism, ModelKind};
use prism_contract::{call_command, deploy_command, vm, ContractEvent, ContractPrism};
use prism_nft::{create_collection_command, owner_of, NftCommand, NftEvent, NftPrism};
use prism_storage::StoragePrism;
use prism_token::{
    balance_of, create_command as token_create, transfer_command as token_transfer, TokenEvent,
    TokenPrism,
};
use veilux_consensus::{Aurora, ConsensusConfig, Validator, ValidatorSet};
use veilux_kernel::{Cascade, Hash, PartyId, Visibility, PROTOCOL_VERSION, TOKEN_TICKER};
use veilux_store::Store;
use veilux_veil::{
    audit_open, grant_disclosure, AuditableEntry, GrantScope, PartyIdentity, ViewKeyring,
};

use node::Node;

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).cloned().unwrap_or_else(|| "info".to_string());
    setup_logging();
    print_banner();

    match cmd.as_str() {
        "info" => cmd_info(),
        "demo" => cmd_demo(),
        "run" => cmd_run(args.get(2).map(|s| s.as_str()).unwrap_or("./veilux-data")),
        "serve" => cmd_serve(&args),
        "validator" => cmd_validator(&args),
        "version" | "--version" | "-V" => {
            println!("veilux {VERSION} ({PROTOCOL_VERSION})");
            Ok(())
        }
        other => {
            eprintln!(
                "unknown command: {other}\n\nUSAGE:\n  veilux info          show kernel + installed prisms\n  veilux demo          run the end-to-end demo\n  veilux run [dir]     run a persistent single node\n  veilux serve [addr] [dir]   run a dev RPC node (default 127.0.0.1:8645)\n  veilux validator --name N --seed S --listen ADDR [--peer name:seed] [--bootstrap ADDR] [--datadir DIR]\n  veilux version       print version"
            );
            std::process::exit(1);
        }
    }
}

fn cmd_serve(args: &[String]) -> Result<()> {
    use std::sync::Arc;
    use tokio::sync::Mutex;

    let addr = args
        .iter()
        .position(|a| a == "--addr")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .or_else(|| args.get(2).filter(|s| s.contains(':')).cloned())
        .unwrap_or_else(|| "127.0.0.1:8645".to_string());
    let datadir = args
        .iter()
        .position(|a| a == "--datadir")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "./veilux-dev-data".to_string());

    let mut cascade = Cascade::new();
    cascade
        .install(Box::new(AiPrism::new()))
        .install(Box::new(StoragePrism::new()))
        .install(Box::new(TokenPrism::new()))
        .install(Box::new(NftPrism::new()))
        .install(Box::new(ContractPrism::new()));

    let store = Store::open(&datadir)?;
    let node = Node::with_store(PartyId::new("dev-node"), cascade, store)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    println!("VEILUX dev RPC node");
    println!("  endpoint : http://{addr}");
    println!("  datadir  : {datadir}");
    println!("  height   : #{}", node.head().height);
    println!("\nTry: curl -s http://{addr} -d '{{\"jsonrpc\":\"2.0\",\"method\":\"veilux_nodeInfo\",\"params\":{{}},\"id\":1}}'");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let shared = Arc::new(Mutex::new(node));
        rpc_service::serve_rpc(shared, addr).await
    })?;
    Ok(())
}

fn cmd_validator(args: &[String]) -> Result<()> {
    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let name = get("--name").unwrap_or_else(|| "validator-0".to_string());
    let seed_str = get("--seed").unwrap_or_else(|| name.clone());
    let listen = get("--listen").unwrap_or_else(|| "127.0.0.1:30420".to_string());
    let datadir = get("--datadir").unwrap_or_else(|| format!("./veilux-data-{name}"));
    let interval = get("--interval")
        .and_then(|s| s.parse().ok())
        .unwrap_or(3u64);

    let bootstrap: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--bootstrap")
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect();

    let peers: Vec<(String, [u8; 32])> = args
        .iter()
        .enumerate()
        .filter(|(_, a)| *a == "--peer")
        .filter_map(|(i, _)| args.get(i + 1))
        .filter_map(|spec| {
            let (n, s) = spec.split_once(':')?;
            Some((n.to_string(), seed_from(s)))
        })
        .collect();

    let cfg = validator_loop::ValidatorConfig {
        name,
        seed: seed_from(&seed_str),
        datadir,
        listen_addr: listen,
        bootstrap,
        peers,
        block_interval_secs: interval,
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(validator_loop::run_validator(cfg))
}

fn seed_from(s: &str) -> [u8; 32] {
    let mut seed = [0u8; 32];
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate().take(32) {
        seed[i] = *b;
    }
    if bytes.is_empty() {
        seed[0] = 1;
    }
    seed
}

fn print_banner() {
    eprintln!(
        "\n  \x1b[36m▌ ▐·▄▄▄ .▪  ▄▄▌  ▄• ▄▌▐▄• ▄ \x1b[0m\n \x1b[36m▪█·█▌▀▄.▀·██ ██•  █▪██▌ █▌█▌▪\x1b[0m   VEILUX v{VERSION}\n \x1b[36m▐█▐█•▐▀▀▪▄▐█·██▪  █▌▐█▌ ·██· \x1b[0m   {PROTOCOL_VERSION}\n  \x1b[36m███ ▐█▄▄▌▐█▌·▐█▌▐▌▐█▄█▌▪▐█·█▌\x1b[0m   featherweight · privacy-first · AI-native\n  \x1b[36m. ▀  ▀▀▀ ▀▀▀ .▀▀▀  ▀▀▀ •▀▀ ▀▀\x1b[0m\n"
    );
}

fn build_node(proposer: &str) -> Node {
    let mut cascade = Cascade::new();
    cascade
        .install(Box::new(AiPrism::new()))
        .install(Box::new(StoragePrism::new()))
        .install(Box::new(TokenPrism::new()))
        .install(Box::new(NftPrism::new()))
        .install(Box::new(ContractPrism::new()));
    Node::new(PartyId::new(proposer), cascade)
}

fn cmd_run(datadir: &str) -> Result<()> {
    info!(datadir, "starting persistent VEILUX node");

    let mut cascade = Cascade::new();
    cascade
        .install(Box::new(AiPrism::new()))
        .install(Box::new(StoragePrism::new()))
        .install(Box::new(TokenPrism::new()))
        .install(Box::new(NftPrism::new()))
        .install(Box::new(ContractPrism::new()));

    let store = Store::open(datadir)?;
    let proposer = PartyId::new("validator-0");
    let mut node = Node::with_store(proposer.clone(), cascade, store)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let mut validators = ValidatorSet::new();
    let id = PartyIdentity::from_seed("validator-0", &[1u8; 32]);
    validators.add(Validator::new(
        proposer.clone(),
        id.public_key().to_vec(),
        100,
    ));
    let aurora = Aurora::new(
        ConsensusConfig::default(),
        validators,
        Some(proposer.clone()),
    );

    let head = node.head();
    info!(
        height = head.height,
        hash = %head.hash(),
        blocks_on_disk = node.blocks.len(),
        "chain loaded from disk"
    );
    println!("VEILUX node running (persistent).");
    println!("  datadir       : {datadir}");
    println!("  chain height  : #{}", node.head().height);
    println!("  head hash     : {}", node.head().hash());
    println!("  state root    : {}", node.state.root());
    println!(
        "  proposer slot : {} (consensus: Aurora BFT)",
        aurora
            .proposer_for(node.head().height + 1, 0)
            .map(|p| p.0)
            .unwrap_or_default()
    );

    let demo_id = PartyIdentity::from_seed("validator-0", &[1u8; 32]);
    let store_party = demo_id.party().clone();
    let next_nonce = node.nonces.get(&store_party).map(|n| n + 1).unwrap_or(0);
    let put = prism_storage::put_command(
        store_party,
        Visibility::Public,
        next_nonce,
        "heartbeat",
        format!("block-{}", node.head().height + 1).into_bytes(),
    );
    node.submit_signed(demo_id.sign(put))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let summary = node
        .produce_block()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    info!(height = summary.height, hash = %summary.hash, "block produced and persisted");
    println!("\nproduced + persisted block #{}", summary.height);
    println!("re-run `veilux run {datadir}` to see the chain grow and reload from disk.");
    Ok(())
}

fn cmd_info() -> Result<()> {
    let node = build_node("validator-0");
    println!("VEILUX — featherweight · privacy-first · AI-native");
    println!("  protocol : {PROTOCOL_VERSION}");
    println!("  token    : {TOKEN_TICKER}");
    println!(
        "  head     : #{} {}",
        node.head().height,
        node.head().hash()
    );
    println!("\ninstalled prisms:");
    for p in node.cascade.installed() {
        println!("  - {:<10} v{:<4} {}", p.name, p.version, p.description);
    }
    Ok(())
}

fn cmd_demo() -> Result<()> {
    info!("starting VEILUX demo");

    let mut node = build_node("validator-0");

    let alice_id = PartyIdentity::from_seed("alice", &[7u8; 32]);
    let alice_ring = ViewKeyring::from_passphrase(PartyId::new("alice"), "alice-secret-seed");
    let bob_ring = ViewKeyring::from_passphrase(PartyId::new("bob"), "bob-secret-seed");
    node.host_party(alice_ring.clone());
    node.host_party(bob_ring);

    let reg = register_command(
        PartyId::new("alice"),
        Visibility::Public,
        0,
        "sentiment-v1",
        ModelKind::Classification,
        Hash::digest(b"model-weights-blob"),
        "1.0",
        3,
    );
    node.submit_signed(alice_id.sign(reg))?;
    let s = node.produce_block()?;
    info!(height = s.height, events = s.events, hash = %s.hash, cost = s.total_cost, "block produced (model registration)");

    let model_id = node
        .sub_ledger(&PartyId::new("alice"))
        .unwrap()
        .entries_for_prism("ai")
        .find_map(
            |e| match serde_json::from_slice::<AiEvent>(&e.event.payload).ok()? {
                AiEvent::ModelRegistered { model_id, .. } => Some(model_id),
                _ => None,
            },
        )
        .expect("model registered");
    println!("registered model: {}", model_id);

    let infer = infer_command(
        PartyId::new("alice"),
        Visibility::Parties(vec![PartyId::new("alice")]),
        1,
        model_id,
        b"i love this product".to_vec(),
    );
    let est = node.estimate(&infer)?;
    println!("estimated inference cost: {est} {}", TOKEN_TICKER);
    node.submit_signed(alice_id.sign(infer))?;
    let s2 = node.produce_block()?;
    info!(
        height = s2.height,
        events = s2.events,
        views = s2.views_delivered,
        "block produced (private inference)"
    );

    let alice_view = node.sub_ledger(&PartyId::new("alice")).unwrap();
    let bob_view = node.sub_ledger(&PartyId::new("bob")).unwrap();
    println!("\n--- privacy check (VeilLedger) ---");
    println!("global events_root (all nodes agree): {}", s2.events_root);
    println!(
        "alice can see {} ai event(s)",
        alice_view.entries_for_prism("ai").count()
    );
    println!(
        "bob   can see {} ai event(s)",
        bob_view.entries_for_prism("ai").count()
    );

    let inference_entry = alice_view.entries_for_prism("ai").find(|e| {
        matches!(
            serde_json::from_slice::<AiEvent>(&e.event.payload),
            Ok(AiEvent::InferenceCommitted { .. })
        )
    });

    if let Some(entry) = inference_entry {
        let sealed = alice_ring.seal(&entry.event)?;
        let auditable = vec![AuditableEntry {
            height: entry.height,
            prism: "ai",
            view: &sealed,
        }];
        let grant = grant_disclosure(
            &alice_ring,
            PartyId::new("regulator"),
            GrantScope::Prism("ai".into()),
            &auditable,
            "scheduled supervisory review",
        );
        let disclosed = audit_open(&grant, &[sealed])?;
        println!("\n--- selective disclosure (banking-grade) ---");
        println!(
            "regulator grant covers {} event(s), basis: {}",
            grant.disclosed_count(),
            grant.justification
        );
        println!(
            "regulator decrypted {} AI event(s) within scope",
            disclosed.len()
        );
    }

    println!("\nbob sees the transaction happened (commitment in root) but NOT its contents.");
    println!("state_root: {}", s2.state_root);

    demo_token(&mut node, &alice_id)?;
    demo_nft(&mut node, &alice_id)?;
    demo_contract(&mut node, &alice_id)?;

    info!("demo complete");
    Ok(())
}

fn demo_token(node: &mut Node, alice_id: &PartyIdentity) -> Result<()> {
    println!("\n--- token prism (fungible, ERC-20-like) ---");
    let create = token_create(
        PartyId::new("alice"),
        Visibility::Public,
        2,
        "Gold Coin",
        "GLD",
        18,
        1_000_000,
        true,
    );
    node.submit_signed(alice_id.sign(create))?;
    let s = node.produce_block()?;

    let token_id = node
        .sub_ledger(&PartyId::new("alice"))
        .unwrap()
        .entries_for_prism("token")
        .find_map(
            |e| match serde_json::from_slice::<TokenEvent>(&e.event.payload).ok()? {
                TokenEvent::Created { token_id, .. } => Some(token_id),
                _ => None,
            },
        )
        .expect("token created");

    let transfer = token_transfer(
        PartyId::new("alice"),
        Visibility::Public,
        3,
        token_id,
        PartyId::new("bob"),
        250_000,
    );
    node.submit_signed(alice_id.sign(transfer))?;
    node.produce_block()?;

    info!(height = s.height, "token created + transfer committed");
    println!("token {} created, supply 1,000,000 GLD", token_id);
    println!(
        "alice balance: {}",
        balance_of(&node.state, &token_id, &PartyId::new("alice"))
    );
    println!(
        "bob   balance: {}",
        balance_of(&node.state, &token_id, &PartyId::new("bob"))
    );
    Ok(())
}

fn demo_nft(node: &mut Node, alice_id: &PartyIdentity) -> Result<()> {
    println!("\n--- nft prism (non-fungible, ERC-721-like) ---");
    let create = create_collection_command(
        PartyId::new("alice"),
        Visibility::Public,
        4,
        "Veil Art",
        "VART",
        Some(100),
    );
    node.submit_signed(alice_id.sign(create))?;
    node.produce_block()?;

    let collection_id = node
        .sub_ledger(&PartyId::new("alice"))
        .unwrap()
        .entries_for_prism("nft")
        .find_map(
            |e| match serde_json::from_slice::<NftEvent>(&e.event.payload).ok()? {
                NftEvent::CollectionCreated { collection_id, .. } => Some(collection_id),
                _ => None,
            },
        )
        .expect("collection created");

    let mint = veilux_kernel::Command {
        prism: "nft".into(),
        submitter: PartyId::new("alice"),
        visibility: Visibility::Public,
        payload: serde_json::to_vec(&NftCommand::Mint {
            collection_id,
            to: PartyId::new("alice"),
            metadata_uri: "ipfs://Qm.../1.json".into(),
            content_hash: Hash::digest(b"artwork-1"),
        })?,
        nonce: 5,
    };
    node.submit_signed(alice_id.sign(mint))?;
    node.produce_block()?;

    println!("collection {} created (max 100)", collection_id);
    println!(
        "token #0 owner: {:?}",
        owner_of(&node.state, &collection_id, 0)
    );
    Ok(())
}

fn demo_contract(node: &mut Node, alice_id: &PartyIdentity) -> Result<()> {
    println!("\n--- contract prism (PhotonVM) ---");

    let mut code = vec![vm::PUSH8];
    code.extend_from_slice(&111u64.to_be_bytes());
    code.push(vm::PUSH8);
    code.extend_from_slice(&222u64.to_be_bytes());
    code.push(vm::ADD);
    code.push(vm::RETURN);

    let deploy = deploy_command(PartyId::new("alice"), Visibility::Public, 6, code);
    node.submit_signed(alice_id.sign(deploy))?;
    node.produce_block()?;

    let address = node
        .sub_ledger(&PartyId::new("alice"))
        .unwrap()
        .entries_for_prism("contract")
        .find_map(
            |e| match serde_json::from_slice::<ContractEvent>(&e.event.payload).ok()? {
                ContractEvent::Deployed { address, .. } => Some(address),
                _ => None,
            },
        )
        .expect("contract deployed");

    let call = call_command(
        PartyId::new("alice"),
        Visibility::Public,
        7,
        address,
        vec![],
        0,
        1_000_000,
    );
    node.submit_signed(alice_id.sign(call))?;
    node.produce_block()?;

    let result = node
        .sub_ledger(&PartyId::new("alice"))
        .unwrap()
        .entries_for_prism("contract")
        .filter_map(|e| serde_json::from_slice::<ContractEvent>(&e.event.payload).ok())
        .find_map(|e| match e {
            ContractEvent::Called {
                return_value,
                gas_used,
                ..
            } => Some((return_value, gas_used)),
            _ => None,
        });

    println!("contract deployed at {}", address);
    if let Some((ret, gas)) = result {
        println!("call result: 111 + 222 = {:?} (gas {})", ret, gas);
    }
    Ok(())
}

fn setup_logging() {
    use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

    let filter = EnvFilter::try_from_env("VEILUX_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("veilux=info,prism_ai=info,prism_token=info,prism_nft=info,prism_contract=info,veilux_veil=info,info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            fmt::layer()
                .with_target(true)
                .with_level(true)
                .with_ansi(true)
                .compact(),
        )
        .init();
}
