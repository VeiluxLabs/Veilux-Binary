use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use tokio::sync::Mutex;
use tracing::{debug, info, warn};
use veilux_consensus::{Aurora, ConsensusConfig, Validator, ValidatorSet, Vote};
use veilux_kernel::{Cascade, PartyId, SignedCommand, Visibility};
use veilux_network::{AuthConfig, NetConfig, NetHandle, NetMessage, Network, PeerKey, ViewChange};
use veilux_store::Store;
use veilux_veil::PartyIdentity;

use crate::driver::{Action, Outbound, Phase, RoundMachine};
use crate::ingress::{serve_ingress, IngressState};
use crate::node::Node;
use crate::slash_watch::EquivocationWatch;
use crate::viewsync::ViewCoordinator;

pub struct ValidatorConfig {
    pub name: String,
    pub seed: [u8; 32],
    pub datadir: String,
    pub listen_addr: String,
    pub bootstrap: Vec<String>,
    pub peers: Vec<(String, [u8; 32])>,
    pub block_interval_secs: u64,
    pub secure: bool,
    pub ip_allowlist: Vec<std::net::IpAddr>,
    pub genesis: Option<crate::genesis::ChainSpec>,
    pub rpc_addr: Option<String>,
    pub host_parties: Vec<(String, String)>,
}

const VIEW_TIMEOUT_TICKS: u32 = 3;

fn validator_set(me: &(String, [u8; 32]), peers: &[(String, [u8; 32])]) -> ValidatorSet {
    let mut vs = ValidatorSet::new();
    let id = PartyIdentity::from_seed(&me.0, &me.1);
    vs.add(Validator::new(
        PartyId::new(&me.0),
        id.public_key().to_vec(),
        100,
    ));
    for (name, seed) in peers {
        let pid = PartyIdentity::from_seed(name, seed);
        vs.add(Validator::new(
            PartyId::new(name),
            pid.public_key().to_vec(),
            100,
        ));
    }
    vs
}

struct Engine {
    node: Node,
    aurora: Aurora,
    identity: PartyIdentity,
    me: PartyId,
    height: u64,
    view: u32,
    stale_ticks: u32,
    machine: RoundMachine,
    viewsync: ViewCoordinator,
    slash_watch: EquivocationWatch,
}

impl Engine {
    fn reset_height(&mut self) {
        self.height = self.node.head().height + 1;
        self.view = 0;
        self.stale_ticks = 0;
        self.machine = RoundMachine::new_round(self.height, self.view);
        self.viewsync.reset(self.height);
        self.slash_watch.prune_below(self.height.saturating_sub(8));
    }

    fn adopt_view(&mut self, view: u32) {
        if view > self.view {
            self.view = view;
            self.stale_ticks = 0;
            self.machine = RoundMachine::new_round(self.height, view);
            info!(
                height = self.height,
                view,
                proposer = ?self.aurora.proposer_for(self.height, view),
                "adopted new view (quorum failover)"
            );
        }
    }
}

pub async fn run_validator(cfg: ValidatorConfig) -> Result<()> {
    let me = PartyId::new(&cfg.name);
    let identity = PartyIdentity::from_seed(&cfg.name, &cfg.seed);

    let mut cascade = Cascade::new();
    cascade
        .install(Box::new(prism_storage::StoragePrism::new()))
        .install(Box::new(prism_token::TokenPrism::new()))
        .install(Box::new(prism_nft::NftPrism::new()))
        .install(Box::new(prism_contract::ContractPrism::new()))
        .install(Box::new(prism_ai::AiPrism::new()))
        .install(Box::new(prism_bridge::BridgePrism::new()))
        .install(Box::new(prism_staking::StakingPrism::new()))
        .install(Box::new(prism_oracle::OraclePrism::new()))
        .install(Box::new(prism_confidential::ConfidentialPrism::new()));

    let store = Store::open(&cfg.datadir)?;
    let mut node =
        Node::with_store(me.clone(), cascade, store).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    if let Some(spec) = &cfg.genesis {
        let seeded = node
            .seed_genesis_state(|state| {
                spec.seed(state)
                    .map_err(|e| crate::node::NodeError::Store(e.to_string()))
            })
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        node.fee_policy = spec.fee_policy();
        node.chain_id = spec.chain_id;
        if seeded {
            info!(
                token = %spec.token_symbol,
                supply = %spec.total_supply_whole(),
                "genesis native token seeded"
            );
        }
    }

    for (name, pass) in &cfg.host_parties {
        node.host_party(veilux_veil::ViewKeyring::from_passphrase(
            PartyId::new(name),
            pass,
        ));
        info!(party = %name, "hosting confidential-execution stakeholder");
    }

    let vset = validator_set(&(cfg.name.clone(), cfg.seed), &cfg.peers);
    let aurora = Aurora::new(ConsensusConfig::default(), vset.clone(), Some(me.clone()));

    let auth = if cfg.secure {
        let mut peers: Vec<PeerKey> = Vec::with_capacity(cfg.peers.len());
        for (name, seed) in &cfg.peers {
            peers.push(PeerKey {
                party: name.clone(),
                public_key: PartyIdentity::from_seed(name, seed).public_key().to_vec(),
            });
        }
        Some(AuthConfig {
            party: cfg.name.clone(),
            secret_seed: cfg.seed,
            peers,
            ip_allowlist: cfg.ip_allowlist.clone(),
        })
    } else {
        None
    };

    let net = Network::spawn(NetConfig {
        node_id: cfg.name.clone(),
        listen_addr: cfg.listen_addr.clone(),
        bootstrap: cfg.bootstrap.clone(),
        auth,
    });
    let mut net = net;

    info!(
        validator = %cfg.name,
        validators = vset.active_count(),
        listen = %cfg.listen_addr,
        secure = cfg.secure,
        allowlisted_ips = cfg.ip_allowlist.len(),
        "validator node online"
    );

    let start_height = node.head().height + 1;
    let head_height = Arc::new(Mutex::new(node.head().height));
    let mut eng = Engine {
        node,
        aurora,
        identity,
        me,
        height: start_height,
        view: 0,
        stale_ticks: 0,
        machine: RoundMachine::new_round(start_height, 0),
        viewsync: ViewCoordinator::new(start_height),
        slash_watch: EquivocationWatch::new(),
    };

    let (ingress_tx, mut ingress_rx) = tokio::sync::mpsc::unbounded_channel::<SignedCommand>();
    let (private_tx, mut private_rx) =
        tokio::sync::mpsc::unbounded_channel::<veilux_veil::PrivateEnvelope>();
    if let Some(rpc_addr) = cfg.rpc_addr.clone() {
        let (chain_id, network) = cfg
            .genesis
            .as_ref()
            .map(|g| (g.chain_id, g.network.clone()))
            .unwrap_or((1, "veilux-dev".to_string()));
        let state = IngressState {
            tx: ingress_tx,
            private_tx,
            height: Arc::clone(&head_height),
            network,
            chain_id,
        };
        tokio::spawn(async move {
            if let Err(e) = serve_ingress(rpc_addr, state).await {
                warn!(error = %e, "ingress RPC stopped");
            }
        });
    }

    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.block_interval_secs.max(1)));
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                on_tick(&mut eng, &net).await;
                *head_height.lock().await = eng.node.head().height;
            }
            maybe_msg = net.inbound.recv() => {
                let Some(msg) = maybe_msg else { break; };
                handle_message(msg, &net, &mut eng).await;
                *head_height.lock().await = eng.node.head().height;
            }
            Some(signed) = ingress_rx.recv() => {
                match eng.node.submit_signed(signed.clone()) {
                    Ok(()) => {
                        let _ = net.net.broadcast(&NetMessage::Command(Box::new(signed)));
                    }
                    Err(e) => debug!(error = %e, "rejected ingress command"),
                }
            }
            Some(envelope) = private_rx.recv() => {
                match eng.node.apply_private_envelope(&envelope) {
                    Ok(out) => {
                        info!(
                            commitment = %out.commitment,
                            executed = out.executed,
                            "confidential envelope applied; gossiping"
                        );
                        let _ = net.net.broadcast(&NetMessage::Private(Box::new(envelope)));
                        if let Some(att) = out.attestation {
                            let _ = net.net.broadcast(&NetMessage::PrivateRoot(Box::new(att)));
                        }
                    }
                    Err(e) => debug!(error = %e, "rejected ingress private envelope"),
                }
            }
            _ = &mut shutdown => {
                info!("shutdown requested");
                break;
            }
        }
    }

    Ok(())
}

async fn on_tick(eng: &mut Engine, net: &NetHandle) {
    if eng.node.head().height >= eng.height {
        eng.reset_height();
    } else if eng.machine.phase != Phase::Committed {
        eng.stale_ticks += 1;
        if eng.stale_ticks >= VIEW_TIMEOUT_TICKS {
            eng.stale_ticks = 0;
            let next_view = eng.view + 1;
            let vc = make_view_change(&eng.identity, eng.height, next_view);
            if let Some(q) = eng.viewsync.record(
                eng.height,
                next_view,
                eng.me.clone(),
                &eng.aurora.validators,
            ) {
                eng.adopt_view(q);
            }
            let _ = net.net.broadcast(&NetMessage::ViewChange(Box::new(vc)));
            debug!(
                height = eng.height,
                proposed_view = next_view,
                "broadcasting view-change vote"
            );
        }
    }

    if eng.aurora.is_local_proposer(eng.height, eng.view) && eng.machine.phase != Phase::Committed {
        match build_or_reuse_block(&mut eng.node, &eng.identity, eng.height, &eng.machine) {
            Ok(block) => {
                let me = eng.me.clone();
                let acts = eng.machine.on_local_proposal(block, &me, &mut eng.aurora);
                run_actions(eng, net, acts).await;
            }
            Err(e) => warn!(error = %e, "failed to build block"),
        }
    }

    let my_votes: Vec<Vote> = eng.machine.my_votes().to_vec();
    for vote in &my_votes {
        let signed = sign_vote(&eng.identity, vote.clone());
        let _ = net.net.broadcast(&NetMessage::Vote(Box::new(signed)));
    }
}

async fn handle_message(msg: NetMessage, net: &NetHandle, eng: &mut Engine) {
    let parent = eng.node.head().hash();
    match msg {
        NetMessage::Proposal { block, .. } => {
            let me = eng.me.clone();
            let acts = eng
                .machine
                .on_proposal(*block, &me, &mut eng.aurora, parent);
            run_actions(eng, net, acts).await;
        }
        NetMessage::Vote(vote) => {
            detect_equivocation(eng, net, &vote).await;
            let me = eng.me.clone();
            let acts = eng.machine.on_vote(*vote, &me, &mut eng.aurora);
            run_actions(eng, net, acts).await;
        }
        NetMessage::ViewChange(vc) => {
            if verify_view_change(&vc, &eng.aurora.validators) {
                if let Some(q) = eng.viewsync.record(
                    vc.height,
                    vc.view,
                    vc.voter.clone(),
                    &eng.aurora.validators,
                ) {
                    eng.adopt_view(q);
                }
            }
        }
        NetMessage::Command(signed) => {
            if let Err(e) = eng.node.submit_signed(*signed) {
                debug!(error = %e, "rejected gossiped command");
            }
        }
        NetMessage::Private(envelope) => {
            let commitment = envelope.commitment;
            match eng.node.apply_private_envelope(&envelope) {
                Ok(out) => {
                    if out.executed {
                        info!(commitment = %commitment, "confidential envelope executed (stakeholder)");
                        if let Some(att) = out.attestation {
                            let _ = net.net.broadcast(&NetMessage::PrivateRoot(Box::new(att)));
                        }
                    } else {
                        debug!(commitment = %commitment, "confidential envelope recorded (non-stakeholder)");
                    }
                }
                Err(e) => debug!(error = %e, "rejected gossiped private envelope"),
            }
        }
        NetMessage::PrivateRoot(att) => {
            let commitment = att.commitment;
            if let Some(proof) = eng.node.private_divergence_proof(&att) {
                warn!(
                    commitment = %commitment,
                    offender = %proof.offender,
                    "PRIVATE-ROOT DIVERGENCE: stakeholder signed two conflicting roots; submitting slash"
                );
                submit_private_slash(eng, net, proof).await;
            }
            match eng.node.record_attestation(*att) {
                veilux_veil::AttestationOutcome::Divergence { existing, incoming } => {
                    warn!(
                        commitment = %commitment,
                        %existing,
                        %incoming,
                        "PRIVATE-ROOT DIVERGENCE: stakeholders disagree on confidential state"
                    );
                    let frauds = eng.node.private_quorum_fraud_proofs(&commitment);
                    for proof in frauds {
                        warn!(
                            commitment = %commitment,
                            offender = %proof.offender,
                            "quorum arbitration: minority signer contradicts the majority root; submitting slash"
                        );
                        submit_quorum_slash(eng, net, proof).await;
                    }
                }
                veilux_veil::AttestationOutcome::Recorded => {
                    debug!(commitment = %commitment, "peer private-root attestation recorded (agrees)");
                }
                _ => {}
            }
        }
        NetMessage::Block(block) => {
            let our_head = eng.node.head().height;
            if block.height > our_head + 1 {
                let _ = net.net.broadcast(&NetMessage::RequestBlocks {
                    from_height: our_head + 1,
                });
            } else if eng.node.accept_external_block(*block).unwrap_or(false) {
                eng.reset_height();
            }
        }
        NetMessage::RequestBlocks { from_height } => {
            let blocks = eng.node.blocks_from(from_height, 256);
            if !blocks.is_empty() {
                let _ = net.net.broadcast(&NetMessage::Blocks { blocks });
            }
        }
        NetMessage::Blocks { blocks } => {
            let mut applied = 0usize;
            for b in blocks {
                if eng.node.accept_external_block(b).unwrap_or(false) {
                    applied += 1;
                }
            }
            if applied > 0 {
                info!(
                    applied,
                    head = eng.node.head().height,
                    "synced blocks from peer"
                );
                eng.reset_height();
            }
        }
        NetMessage::Hello { .. } => {}
    }
}

async fn run_actions(eng: &mut Engine, net: &NetHandle, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Broadcast(Outbound::Proposal { round, block }) => {
                let _ = net.net.broadcast(&NetMessage::Proposal { round, block });
            }
            Action::Broadcast(Outbound::Vote(vote)) => {
                let signed = sign_vote(&eng.identity, *vote);
                let _ = net.net.broadcast(&NetMessage::Vote(Box::new(signed)));
            }
            Action::Commit(block_hash) => {
                if let Some(block) = eng.machine.block(&block_hash).cloned() {
                    match eng.node.commit_block(block) {
                        Ok(summary) => {
                            info!(height = summary.height, hash = %summary.hash, "block committed + persisted");
                            let head = eng.node.head().clone();
                            let _ = net.net.broadcast(&NetMessage::Block(Box::new(head)));
                            eng.reset_height();
                        }
                        Err(e) => warn!(error = %e, "commit failed"),
                    }
                }
            }
            Action::None => {}
        }
    }
}

fn build_or_reuse_block(
    node: &mut Node,
    identity: &PartyIdentity,
    height: u64,
    machine: &RoundMachine,
) -> Result<veilux_kernel::Block> {
    if let Some(existing) = machine.own_proposed_block() {
        return Ok(existing.clone());
    }
    build_local_block(node, identity, height)?.ok_or_else(|| anyhow::anyhow!("no block built"))
}

fn build_local_block(
    node: &mut Node,
    identity: &PartyIdentity,
    height: u64,
) -> Result<Option<veilux_kernel::Block>> {
    let nonce = node
        .nonces
        .get(identity.party())
        .map(|n| n + 1)
        .unwrap_or(0);
    let heartbeat = prism_storage::put_command(
        identity.party().clone(),
        Visibility::Public,
        nonce,
        "block-heartbeat",
        format!("h{height}").into_bytes(),
    );
    node.submit_signed(identity.sign_for_chain(heartbeat, node.chain_id))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let block = node
        .assemble_block()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(Some(block))
}

fn sign_vote(identity: &PartyIdentity, mut vote: Vote) -> Vote {
    if vote.voter == *identity.party() && vote.signature.is_empty() {
        vote.signature = identity.sign_bytes(&vote.signing_bytes());
    }
    vote
}

async fn submit_private_slash(
    eng: &mut Engine,
    net: &NetHandle,
    proof: prism_staking::EquivocationProof,
) {
    let nonce = eng
        .node
        .nonces
        .get(eng.identity.party())
        .map(|n| n + 1)
        .unwrap_or(0);
    let cmd = prism_staking::staking_command(
        eng.me.clone(),
        Visibility::Public,
        nonce,
        eng.height,
        &prism_staking::StakingCommand::Slash { proof },
    );
    let signed = eng.identity.sign_for_chain(cmd, eng.node.chain_id);
    if eng.node.submit_signed(signed.clone()).is_ok() {
        let _ = net.net.broadcast(&NetMessage::Command(Box::new(signed)));
    }
}

async fn submit_quorum_slash(
    eng: &mut Engine,
    net: &NetHandle,
    proof: prism_staking::QuorumFraudProof,
) {
    let nonce = eng
        .node
        .nonces
        .get(eng.identity.party())
        .map(|n| n + 1)
        .unwrap_or(0);
    let cmd = prism_staking::staking_command(
        eng.me.clone(),
        Visibility::Public,
        nonce,
        eng.height,
        &prism_staking::StakingCommand::SlashQuorum { proof },
    );
    let signed = eng.identity.sign_for_chain(cmd, eng.node.chain_id);
    if eng.node.submit_signed(signed.clone()).is_ok() {
        let _ = net.net.broadcast(&NetMessage::Command(Box::new(signed)));
    }
}

async fn detect_equivocation(eng: &mut Engine, net: &NetHandle, vote: &Vote) {
    let pubkey = match eng.aurora.validators.get(&vote.voter) {
        Some(v) => v.public_key.clone(),
        None => return,
    };
    let Some(proof) = eng.slash_watch.observe(vote, &pubkey) else {
        return;
    };
    warn!(offender = %proof.offender, "equivocation detected; submitting slash evidence");

    let nonce = eng
        .node
        .nonces
        .get(eng.identity.party())
        .map(|n| n + 1)
        .unwrap_or(0);
    let cmd = prism_staking::staking_command(
        eng.me.clone(),
        Visibility::Public,
        nonce,
        eng.height,
        &prism_staking::StakingCommand::Slash { proof },
    );
    let signed = eng.identity.sign_for_chain(cmd, eng.node.chain_id);
    if eng.node.submit_signed(signed.clone()).is_ok() {
        let _ = net.net.broadcast(&NetMessage::Command(Box::new(signed)));
    }
}

fn make_view_change(identity: &PartyIdentity, height: u64, view: u32) -> ViewChange {
    let mut vc = ViewChange {
        height,
        view,
        voter: identity.party().clone(),
        public_key: identity.public_key().to_vec(),
        signature: vec![],
    };
    vc.signature = identity.sign_bytes(&vc.signing_bytes());
    vc
}

fn verify_view_change(vc: &ViewChange, vset: &ValidatorSet) -> bool {
    if !vset.is_validator(&vc.voter) {
        return false;
    }
    veilux_veil::verify_bytes(&vc.public_key, &vc.signing_bytes(), &vc.signature).is_ok()
}
