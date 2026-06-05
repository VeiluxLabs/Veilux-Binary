use std::time::Duration;

use anyhow::Result;
use tracing::{info, warn};
use veilux_consensus::{Aurora, ConsensusConfig, Validator, ValidatorSet, Vote};
use veilux_kernel::{Cascade, PartyId, Visibility};
use veilux_network::{NetConfig, NetMessage, Network};
use veilux_store::Store;
use veilux_veil::PartyIdentity;

use crate::driver::{Action, Outbound, RoundMachine};
use crate::node::Node;

pub struct ValidatorConfig {
    pub name: String,
    pub seed: [u8; 32],
    pub datadir: String,
    pub listen_addr: String,
    pub bootstrap: Vec<String>,
    pub peers: Vec<(String, [u8; 32])>,
    pub block_interval_secs: u64,
}

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

pub async fn run_validator(cfg: ValidatorConfig) -> Result<()> {
    let me = PartyId::new(&cfg.name);
    let identity = PartyIdentity::from_seed(&cfg.name, &cfg.seed);

    let mut cascade = Cascade::new();
    cascade
        .install(Box::new(prism_storage::StoragePrism::new()))
        .install(Box::new(prism_token::TokenPrism::new()))
        .install(Box::new(prism_nft::NftPrism::new()))
        .install(Box::new(prism_contract::ContractPrism::new()))
        .install(Box::new(prism_ai::AiPrism::new()));

    let store = Store::open(&cfg.datadir)?;
    let mut node =
        Node::with_store(me.clone(), cascade, store).map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let vset = validator_set(&(cfg.name.clone(), cfg.seed), &cfg.peers);
    let mut aurora = Aurora::new(ConsensusConfig::default(), vset.clone(), Some(me.clone()));

    let mut net = Network::spawn(NetConfig {
        node_id: cfg.name.clone(),
        listen_addr: cfg.listen_addr.clone(),
        bootstrap: cfg.bootstrap.clone(),
    });

    info!(
        validator = %cfg.name,
        validators = vset.active_count(),
        listen = %cfg.listen_addr,
        "validator node online"
    );

    let mut height = node.head().height + 1;
    let mut machine = RoundMachine::new_round(height, 0);
    let mut ticker = tokio::time::interval(Duration::from_secs(cfg.block_interval_secs.max(1)));

    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if node.head().height >= height {
                    height = node.head().height + 1;
                    machine = RoundMachine::new_round(height, 0);
                }
                // Height-only proposer selection: all nodes agree on the
                // proposer for a height. The proposer re-proposes every tick
                // until the block commits, which tolerates peers that connect
                // late. (Proposer failover when the leader is offline is a
                // documented follow-up.)
                if aurora.is_local_proposer(height, 0)
                    && machine.phase != crate::driver::Phase::Committed
                {
                    match build_or_reuse_block(&mut node, &identity, height, &machine) {
                        Ok(block) => {
                            let acts = machine.on_local_proposal(block, &me, &mut aurora);
                            for action in acts {
                                dispatch(&net, action, &mut node, &mut machine, &identity, &mut height).await;
                            }
                        }
                        Err(e) => warn!(error = %e, "failed to build block"),
                    }
                }

                // Gossip recovery: re-broadcast our own votes each tick so
                // late-joining or lossy peers eventually receive them.
                let my_votes: Vec<Vote> = machine.my_votes().to_vec();
                for vote in &my_votes {
                    let signed = sign_vote(&identity, vote.clone());
                    let _ = net.net.broadcast(&NetMessage::Vote(Box::new(signed)));
                }
            }

            maybe_msg = net.inbound.recv() => {
                let Some(msg) = maybe_msg else { break; };
                handle_message(msg, &net, &mut node, &mut machine, &mut aurora, &identity, &me, &mut height).await;
                if node.head().height >= height {
                    height = node.head().height + 1;
                    machine = RoundMachine::new_round(height, 0);
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
    node.submit_signed(identity.sign(heartbeat))
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    let block = node
        .assemble_block()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(Some(block))
}

#[allow(clippy::too_many_arguments)]
async fn handle_message(
    msg: NetMessage,
    net: &veilux_network::NetHandle,
    node: &mut Node,
    machine: &mut RoundMachine,
    aurora: &mut Aurora,
    identity: &PartyIdentity,
    me: &PartyId,
    height: &mut u64,
) {
    let parent = node.head().hash();
    match msg {
        NetMessage::Proposal { block, .. } => {
            let actions = machine.on_proposal(*block, me, aurora, parent);
            for a in actions {
                dispatch(net, a, node, machine, identity, height).await;
            }
        }
        NetMessage::Vote(vote) => {
            let actions = machine.on_vote(*vote, me, aurora);
            for a in actions {
                dispatch(net, a, node, machine, identity, height).await;
            }
        }
        NetMessage::Command(signed) => {
            if let Err(e) = node.submit_signed(*signed) {
                warn!(error = %e, "rejected gossiped command");
            }
        }
        NetMessage::Block(block) => {
            let _ = node.accept_external_block(*block);
        }
        _ => {}
    }
}

fn sign_vote(identity: &PartyIdentity, mut vote: Vote) -> Vote {
    if vote.voter == *identity.party() && vote.signature.is_empty() {
        vote.signature = identity.sign_bytes(&vote.signing_bytes());
    }
    vote
}

async fn dispatch(
    net: &veilux_network::NetHandle,
    action: Action,
    node: &mut Node,
    machine: &mut RoundMachine,
    identity: &PartyIdentity,
    height: &mut u64,
) {
    match action {
        Action::Broadcast(Outbound::Proposal { round, block }) => {
            let _ = net.net.broadcast(&NetMessage::Proposal { round, block });
        }
        Action::Broadcast(Outbound::Vote(vote)) => {
            let signed = sign_vote(identity, *vote);
            let _ = net.net.broadcast(&NetMessage::Vote(Box::new(signed)));
        }
        Action::Commit(block_hash) => {
            if let Some(block) = machine.block(&block_hash).cloned() {
                match node.commit_block(block) {
                    Ok(summary) => {
                        info!(height = summary.height, hash = %summary.hash, "block committed + persisted");
                        let _ = net
                            .net
                            .broadcast(&NetMessage::Block(Box::new(node.head().clone())));
                        *height += 1;
                        *machine = RoundMachine::new(*height);
                    }
                    Err(e) => warn!(error = %e, "commit failed"),
                }
            }
        }
        Action::None => {}
    }
}
