use std::sync::Arc;

use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::Mutex;
use tracing::info;
use veilux_kernel::SignedCommand;
use veilux_rpc::server::RpcServer;
use veilux_rpc::types::{codes, RpcRequest, RpcResponse, SubmitResult};
use veilux_veil::PrivateEnvelope;

#[derive(Clone)]
pub struct IngressState {
    pub tx: UnboundedSender<SignedCommand>,
    pub private_tx: UnboundedSender<PrivateEnvelope>,
    pub height: Arc<Mutex<u64>>,
    pub network: String,
    pub chain_id: u64,
}

pub async fn serve_ingress(addr: String, state: IngressState) -> std::io::Result<()> {
    let server = RpcServer::new(addr.clone());
    info!(%addr, "validator tx-ingress RPC online (submit + read-lite)");
    server
        .serve(move |req| {
            let state = state.clone();
            async move { dispatch(state, req).await }
        })
        .await
}

async fn dispatch(state: IngressState, req: RpcRequest) -> RpcResponse {
    let id = req.id.clone();
    match req.method.as_str() {
        "veilux_submit" => {
            let signed: Option<SignedCommand> = req
                .params
                .get("command")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            let Some(signed) = signed else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing 'command'");
            };
            if signed.chain_id != state.chain_id {
                return RpcResponse::err(
                    id,
                    codes::COMMAND_REJECTED,
                    format!(
                        "wrong chain id: command signed for {}, this chain is {}",
                        signed.chain_id, state.chain_id
                    ),
                );
            }
            if veilux_veil::verify_signed(&signed).is_err() {
                return RpcResponse::err(id, codes::COMMAND_REJECTED, "invalid signature");
            }
            if signed.command.submitter.0.contains('/') || signed.command.submitter.0.is_empty() {
                return RpcResponse::err(id, codes::COMMAND_REJECTED, "reserved account");
            }
            let command_id = signed.command.id().to_hex();
            match state.tx.send(signed) {
                Ok(()) => ok(
                    id,
                    SubmitResult {
                        accepted: true,
                        command_id,
                        mempool_len: 0,
                    },
                ),
                Err(_) => {
                    RpcResponse::err(id, codes::INTERNAL_ERROR, "consensus loop unavailable")
                }
            }
        }
        "veilux_submitPrivate" => {
            let envelope: Option<PrivateEnvelope> = req
                .params
                .get("envelope")
                .cloned()
                .or_else(|| Some(req.params.clone()))
                .and_then(|v| serde_json::from_value(v).ok());
            let Some(envelope) = envelope else {
                return RpcResponse::err(id, codes::INVALID_PARAMS, "missing/invalid 'envelope'");
            };
            if !envelope.verify_commitment() {
                return RpcResponse::err(id, codes::COMMAND_REJECTED, "bad commitment");
            }
            let commitment = envelope.commitment.to_hex();
            match state.private_tx.send(envelope) {
                Ok(()) => ok(
                    id,
                    serde_json::json!({ "accepted": true, "commitment": commitment }),
                ),
                Err(_) => {
                    RpcResponse::err(id, codes::INTERNAL_ERROR, "consensus loop unavailable")
                }
            }
        }
        "veilux_blockNumber" => {
            let h = *state.height.lock().await;
            ok(id, h)
        }
        "veilux_chainId" => ok(id, state.chain_id),
        "veilux_nodeInfo" => {
            let h = *state.height.lock().await;
            ok(
                id,
                serde_json::json!({
                    "network": state.network,
                    "chain_id": state.chain_id,
                    "protocol": veilux_kernel::PROTOCOL_VERSION,
                    "token": veilux_kernel::TOKEN_TICKER,
                    "height": h,
                    "role": "validator",
                }),
            )
        }
        other => RpcResponse::err(
            id,
            codes::METHOD_NOT_FOUND,
            format!("validator ingress supports veilux_submit/blockNumber/chainId/nodeInfo; got {other}"),
        ),
    }
}

fn ok<T: serde::Serialize>(id: serde_json::Value, result: T) -> RpcResponse {
    RpcResponse::ok(
        id,
        serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
    )
}
