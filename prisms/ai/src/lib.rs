#[cfg(feature = "ollama")]
pub mod ollama;

use serde::{Deserialize, Serialize};

use veilux_kernel::{
    Command, Event, Hash, PartyId, Prism, PrismError, PrismInfo, PrismOutput, StateTree, Visibility,
};

const MODEL_PREFIX: &str = "ai/models/";
const OFFLOAD_THRESHOLD: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelKind {
    Classification,
    Regression,
    Embedding,
    Text,
}

impl ModelKind {
    fn base_cost(&self) -> u64 {
        match self {
            ModelKind::Classification => 8_000,
            ModelKind::Regression => 6_000,
            ModelKind::Embedding => 12_000,
            ModelKind::Text => 20_000,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelRecord {
    pub model_id: Hash,
    pub name: String,
    pub kind: ModelKind,
    pub weights_hash: Hash,
    pub owner: PartyId,
    pub version: String,
    pub dimensions: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum AiCommand {
    Register {
        name: String,
        kind: ModelKind,
        weights_hash: Hash,
        version: String,
        dimensions: u32,
    },
    Infer {
        model_id: Hash,
        input: Vec<u8>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AiEvent {
    ModelRegistered {
        model_id: Hash,
        name: String,
    },
    InferenceCommitted {
        model_id: Hash,
        input_hash: Hash,
        output_hash: Hash,
        output: Option<Vec<u8>>,
        cost: u64,
    },
}

#[derive(Default)]
pub struct AiPrism;

impl AiPrism {
    pub fn new() -> Self {
        AiPrism
    }

    fn model_key(id: &Hash) -> String {
        format!("{MODEL_PREFIX}{}", id.to_hex())
    }

    fn load_model(state: &StateTree, id: &Hash) -> Result<ModelRecord, PrismError> {
        state
            .get_json::<ModelRecord>(&Self::model_key(id))
            .map_err(|e| PrismError::Internal(e.to_string()))?
            .ok_or_else(|| PrismError::NotFound(format!("model {}", id.to_hex())))
    }

    fn run_inference(model: &ModelRecord, input: &[u8]) -> (Vec<u8>, u64) {
        match model.kind {
            ModelKind::Classification => {
                let n = model.dimensions.max(1) as usize;
                let mut best_class = 0u32;
                let mut best_score = 0u64;
                for c in 0..n {
                    let h = Hash::commit(
                        "ai/clf",
                        &[
                            model.weights_hash.as_bytes(),
                            input,
                            &(c as u32).to_le_bytes(),
                        ],
                    );
                    let score = u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
                    if score > best_score {
                        best_score = score;
                        best_class = c as u32;
                    }
                }
                let confidence = (best_score % 1_000_000) as u32;
                let out = ClassificationOut {
                    class: best_class,
                    confidence,
                };
                (
                    serde_json::to_vec(&out).unwrap_or_default(),
                    model.kind.base_cost(),
                )
            }
            ModelKind::Regression => {
                let h = Hash::commit("ai/reg", &[model.weights_hash.as_bytes(), input]);
                let raw = i64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
                let value = raw % 1_000_000_000;
                let out = RegressionOut { value };
                (
                    serde_json::to_vec(&out).unwrap_or_default(),
                    model.kind.base_cost(),
                )
            }
            ModelKind::Embedding => {
                let dims = model.dimensions.clamp(8, 1024) as usize;
                let mut hasher = blake3::Hasher::new();
                hasher.update(model.weights_hash.as_bytes());
                hasher.update(input);
                let mut xof = hasher.finalize_xof();
                let mut buf = vec![0u8; dims * 4];
                xof.fill(&mut buf);
                let mut vec_f = Vec::with_capacity(dims);
                for chunk in buf.chunks_exact(4) {
                    let v = i32::from_le_bytes(chunk.try_into().unwrap());
                    vec_f.push((v as i64 * 1_000_000 / i32::MAX as i64) as i32);
                }
                let out = EmbeddingOut {
                    dims: dims as u32,
                    values: vec_f,
                };
                (
                    serde_json::to_vec(&out).unwrap_or_default(),
                    model.kind.base_cost() + dims as u64 * 4,
                )
            }
            ModelKind::Text => {
                let h = Hash::commit("ai/text", &[model.weights_hash.as_bytes(), input]);
                let seed = u64::from_le_bytes(h.as_bytes()[..8].try_into().unwrap());
                let templates = [
                    "analysis indicates",
                    "the model concludes",
                    "inference result:",
                    "computed response:",
                ];
                let t = templates[(seed as usize) % templates.len()];
                let text = format!("{t} {}", h.to_hex());
                (text.into_bytes(), model.kind.base_cost())
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct ClassificationOut {
    class: u32,
    confidence: u32,
}
#[derive(Serialize, Deserialize)]
struct RegressionOut {
    value: i64,
}
#[derive(Serialize, Deserialize)]
struct EmbeddingOut {
    dims: u32,
    values: Vec<i32>,
}

impl Prism for AiPrism {
    fn info(&self) -> PrismInfo {
        PrismInfo {
            name: "ai",
            description: "On-chain model registry and deterministic verifiable inference",
            version: "1.0",
        }
    }

    fn handle(&self, command: &Command, state: &mut StateTree) -> Result<PrismOutput, PrismError> {
        let ai_cmd: AiCommand = serde_json::from_slice(&command.payload)
            .map_err(|e| PrismError::InvalidPayload(e.to_string()))?;

        match ai_cmd {
            AiCommand::Register {
                name,
                kind,
                weights_hash,
                version,
                dimensions,
            } => {
                let model_id = Hash::commit(
                    "ai/model-id",
                    &[
                        command.submitter.0.as_bytes(),
                        name.as_bytes(),
                        weights_hash.as_bytes(),
                    ],
                );
                let record = ModelRecord {
                    model_id,
                    name: name.clone(),
                    kind,
                    weights_hash,
                    owner: command.submitter.clone(),
                    version,
                    dimensions,
                };
                state
                    .put_json(AiPrism::model_key(&model_id), &record)
                    .map_err(|e| PrismError::Internal(e.to_string()))?;

                let event = Event {
                    source_command: command.id(),
                    prism: "ai".into(),
                    visibility: command.visibility.clone(),
                    payload: serde_json::to_vec(&AiEvent::ModelRegistered { model_id, name })
                        .unwrap_or_default(),
                };
                Ok(PrismOutput::single(event, 5_000))
            }

            AiCommand::Infer { model_id, input } => {
                let model = AiPrism::load_model(state, &model_id)?;
                let input_hash = Hash::digest(&input);
                let (output, cost) = AiPrism::run_inference(&model, &input);
                let output_hash = Hash::digest(&output);

                let (inline_output, derived) = if output.len() > OFFLOAD_THRESHOLD {
                    let store_payload = StoragePutRequest {
                        key: format!("ai/result/{}", output_hash.to_hex()),
                        data: output.clone(),
                    };
                    let derived_cmd = Command {
                        prism: "storage".into(),
                        submitter: command.submitter.clone(),
                        visibility: command.visibility.clone(),
                        payload: serde_json::to_vec(&store_payload).unwrap_or_default(),
                        nonce: command.nonce,
                    };
                    (None, vec![derived_cmd])
                } else {
                    (Some(output.clone()), vec![])
                };

                let event = Event {
                    source_command: command.id(),
                    prism: "ai".into(),
                    visibility: command.visibility.clone(),
                    payload: serde_json::to_vec(&AiEvent::InferenceCommitted {
                        model_id,
                        input_hash,
                        output_hash,
                        output: inline_output,
                        cost,
                    })
                    .unwrap_or_default(),
                };

                Ok(PrismOutput {
                    events: vec![event],
                    derived_commands: derived,
                    cost,
                })
            }
        }
    }

    fn estimate(&self, command: &Command, _state: &StateTree) -> u64 {
        match serde_json::from_slice::<AiCommand>(&command.payload) {
            Ok(AiCommand::Register { .. }) => 5_000,
            Ok(AiCommand::Infer { .. }) => 15_000,
            Err(_) => 1_000,
        }
    }
}

#[derive(Serialize, Deserialize)]
struct StoragePutRequest {
    key: String,
    data: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
pub fn register_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    name: &str,
    kind: ModelKind,
    weights_hash: Hash,
    version: &str,
    dimensions: u32,
) -> Command {
    let payload = serde_json::to_vec(&AiCommand::Register {
        name: name.to_string(),
        kind,
        weights_hash,
        version: version.to_string(),
        dimensions,
    })
    .unwrap_or_default();
    Command {
        prism: "ai".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

pub fn infer_command(
    submitter: PartyId,
    visibility: Visibility,
    nonce: u64,
    model_id: Hash,
    input: Vec<u8>,
) -> Command {
    let payload = serde_json::to_vec(&AiCommand::Infer { model_id, input }).unwrap_or_default();
    Command {
        prism: "ai".into(),
        submitter,
        visibility,
        payload,
        nonce,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_infer_is_deterministic() {
        let prism = AiPrism::new();
        let mut state = StateTree::new();

        let reg = register_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "mnist",
            ModelKind::Classification,
            Hash::digest(b"weights"),
            "1.0",
            10,
        );
        let out = prism.handle(&reg, &mut state).unwrap();
        assert_eq!(out.events.len(), 1);

        let ev: AiEvent = serde_json::from_slice(&out.events[0].payload).unwrap();
        let model_id = match ev {
            AiEvent::ModelRegistered { model_id, .. } => model_id,
            _ => panic!("expected ModelRegistered"),
        };

        let infer = infer_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            model_id,
            b"image-bytes".to_vec(),
        );
        let r1 = prism.handle(&infer, &mut state).unwrap();
        let r2 = prism.handle(&infer, &mut state).unwrap();
        assert_eq!(
            serde_json::to_vec(&r1.events[0].payload).unwrap(),
            serde_json::to_vec(&r2.events[0].payload).unwrap()
        );
    }

    #[test]
    fn large_embedding_offloads_to_storage() {
        let prism = AiPrism::new();
        let mut state = StateTree::new();
        let reg = register_command(
            PartyId::new("alice"),
            Visibility::Public,
            0,
            "embed",
            ModelKind::Embedding,
            Hash::digest(b"w"),
            "1.0",
            256,
        );
        let out = prism.handle(&reg, &mut state).unwrap();
        let model_id = match serde_json::from_slice::<AiEvent>(&out.events[0].payload).unwrap() {
            AiEvent::ModelRegistered { model_id, .. } => model_id,
            _ => unreachable!(),
        };
        let infer = infer_command(
            PartyId::new("alice"),
            Visibility::Public,
            1,
            model_id,
            b"text".to_vec(),
        );
        let r = prism.handle(&infer, &mut state).unwrap();
        assert_eq!(r.derived_commands.len(), 1);
        assert_eq!(r.derived_commands[0].prism, "storage");
    }
}
