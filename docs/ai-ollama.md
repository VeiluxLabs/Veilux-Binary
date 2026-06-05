# AI Prism + Ollama Integration

VEILUX's AI Prism has two layers:

1. **On-chain, deterministic** (always available): model registry + reproducible
   inference derived from `BLAKE3(weights_hash ‖ input)`. Every validator agrees
   on the result, so it is safe for consensus.
2. **Off-chain, real models via Ollama** (optional `ollama` feature): run actual
   LLMs locally (Llama 3, Mistral, Phi, custom fine-tunes) and bring a
   *commitment* of the result on-chain.

## Why the split?

A blockchain must be deterministic: every node must compute the same state. Real
neural-network inference is not bit-for-bit reproducible across hardware (GPU
nondeterminism, float ordering). So VEILUX never runs Ollama *inside* consensus.
Instead:

```
        off-chain (per node / oracle)             on-chain (consensus)
   ┌──────────────────────────────┐         ┌───────────────────────────┐
   │ OllamaClient.generate(prompt)│  ───►   │ commit (prompt_hash,        │
   │   → real LLM output          │         │         output_hash) via    │
   │   → InferenceAttestation     │         │   ai.infer or a token of    │
   └──────────────────────────────┘         │   proof; verifiable later   │
                                             └───────────────────────────┘
```

The chain stores `prompt_hash` and `output_hash` (and optionally the output),
so anyone can later verify "this model produced this output for this prompt"
without re-running the model on-chain.

## Enabling the feature

```bash
# build the node with Ollama support
cargo build --release --features ollama
```

Configure via environment variables:

| Variable | Default | Meaning |
|----------|---------|---------|
| `VEILUX_OLLAMA_URL` | `http://localhost:11434` | Ollama server URL |
| `VEILUX_OLLAMA_MODEL` | `llama3` | model name/tag |

## Running Ollama

```bash
# install: https://ollama.com
ollama serve
ollama pull llama3            # or mistral, phi3, qwen2, your own
ollama create my-model -f ./Modelfile   # custom model tuned to your needs
```

## Using the client (library)

```rust
use prism_ai::ollama::{OllamaClient, OllamaConfig};

let client = OllamaClient::from_env();           // reads VEILUX_OLLAMA_*
let att = client.generate("Summarize this contract clause: ...")?;
println!("output: {}", att.output);
println!("prompt_hash: {}  output_hash: {}", att.prompt_hash, att.output_hash);

// Embeddings (for semantic search, RAG, similarity on-chain commitments)
let vector = client.embed("tokenized treasury bond")?;
```

The returned `InferenceAttestation { model, prompt_hash, output, output_hash }`
is exactly what you commit on-chain (e.g. via the `ai.infer` command or a
custom Prism), keeping the heavy compute off-chain and the proof on-chain.

## Custom models "sesuai kebutuhan"

Because the model name is just configuration, you can point VEILUX at any Ollama
model: a domain-tuned LLM for legal/financial text, a small fast model for
classification, or your own fine-tune via a `Modelfile`. Swap models without
touching chain code — only `VEILUX_OLLAMA_MODEL` changes.

## Privacy note

When inference runs on sensitive data, submit the resulting `ai` event with
`Visibility::Parties([...])` so the Veil layer seals it to stakeholders only.
The on-chain commitment still lets non-stakeholders verify an inference happened
without seeing the prompt or output (see `docs/privacy-model.md`).

## Roadmap: trustless inference

To make off-chain inference *trustless* (not just attested), see
`docs/roadmap.md` → Verifiable Compute Prism (TEE attestation / ZK proof of
correct execution).
