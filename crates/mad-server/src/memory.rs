use anyhow::{Context, Result, anyhow};
use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokenizers::{PaddingParams, Tokenizer};
use tracing::{info, warn};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemoryItem {
    pub content: String,
    pub embedding: Vec<f32>,
    pub metadata: Value,
    pub timestamp: DateTime<Utc>,
}

struct CandleEmbedding {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl CandleEmbedding {
    fn new(model_id: &str, model_path: Option<String>) -> Result<Self> {
        info!("Initializing Candle Embedding Model ({model_id})...");

        let device = Device::Cpu; // Default to CPU for compatibility

        let (config_filename, tokenizer_filename, weights_filename) = if let Some(path) = model_path
        {
            let path = PathBuf::from(path);
            (
                path.join("config.json"),
                path.join("tokenizer.json"),
                path.join("model.safetensors"),
            )
        } else {
            return Err(anyhow!(
                "Model path not provided. Automatic downloading from HuggingFace is disabled."
            ));
        };

        let config_str = std::fs::read_to_string(config_filename)?;
        let config: Config = serde_json::from_str(&config_str)?;

        let mut tokenizer = Tokenizer::from_file(tokenizer_filename).map_err(anyhow::Error::msg)?;

        // Setup tokenizer padding
        if let Some(pp) = tokenizer.get_padding_mut() {
            pp.strategy = tokenizers::PaddingStrategy::BatchLongest;
        } else {
            let pp = PaddingParams {
                strategy: tokenizers::PaddingStrategy::BatchLongest,
                ..Default::default()
            };
            tokenizer.with_padding(Some(pp));
        }

        // Load weights
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_filename], DType::F32, &device)
                .context("Failed to load weights")?
        };

        let model = BertModel::load(vb, &config).context("Failed to load BertModel")?;

        info!("Candle Embedding Model Initialized.");
        Ok(Self {
            model,
            tokenizer,
            device,
        })
    }

    fn embed(&self, texts: Vec<&str>) -> Result<Vec<Vec<f32>>> {
        let tokens = self
            .tokenizer
            .encode_batch(texts, true)
            .map_err(anyhow::Error::msg)?;

        let token_ids = tokens
            .iter()
            .map(|t| {
                let ids = t.get_ids().to_vec();
                Tensor::new(ids.as_slice(), &self.device)
            })
            .collect::<Result<Vec<_>, _>>()?;

        let token_ids = Tensor::stack(&token_ids, 0)?;
        let token_type_ids = token_ids.zeros_like()?;

        // Extract attention mask
        let attention_mask = tokens
            .iter()
            .map(|t| {
                let mask = t.get_attention_mask().to_vec();
                Tensor::new(mask.as_slice(), &self.device)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let attention_mask = Tensor::stack(&attention_mask, 0)?;

        let embeddings = self
            .model
            .forward(&token_ids, &token_type_ids, Some(&attention_mask))?;

        // Mean pooling
        let (_n_sentence, _n_tokens, _hidden_size) = embeddings.dims3()?;

        let attention_mask_float = attention_mask.to_dtype(DType::F32)?;

        // Expand mask to match embedding dimensions
        let mask_expanded = attention_mask_float.unsqueeze(2)?;

        // Multiply embeddings by mask to zero out padding
        let masked_embeddings = embeddings.broadcast_mul(&mask_expanded)?;

        // Sum along token dimension (dim 1)
        let summed = masked_embeddings.sum(1)?;

        // Count non-padding tokens
        let counts = attention_mask_float.sum(1)?.unsqueeze(1)?;

        // Clamp counts to avoid division by zero (though shouldn't happen with valid input)
        let counts = counts.maximum(&Tensor::new(&[1e-9f32], &self.device)?)?;

        let mean_pooled = (summed / counts)?;

        // L2 Normalize
        let sum_sq = mean_pooled.sqr()?.sum_keepdim(1)?;
        let norm = sum_sq.sqrt()?;
        let norm = norm.maximum(&Tensor::new(&[1e-9f32], &self.device)?)?;
        let normalized_embeddings = (mean_pooled / norm)?;

        let embeddings_vec: Vec<Vec<f32>> = normalized_embeddings.to_vec2()?;
        Ok(embeddings_vec)
    }
}

pub struct MemoryStore {
    model: CandleEmbedding,
    memories: Vec<MemoryItem>,
}

impl MemoryStore {
    pub fn new(model_id: &str, model_path: Option<String>) -> Result<Self> {
        let model = CandleEmbedding::new(model_id, model_path)?;

        Ok(Self {
            model,
            memories: Vec::new(),
        })
    }

    pub fn add_memory(&mut self, content: &str, metadata: Value) -> Result<()> {
        let embeddings = self.model.embed(vec![content])?;

        if let Some(embedding) = embeddings.first() {
            let memory = MemoryItem {
                content: content.to_string(),
                embedding: embedding.clone(),
                metadata,
                timestamp: Utc::now(),
            };
            self.memories.push(memory);
            info!("Added memory: '{}'", content);
        } else {
            warn!("Failed to generate embedding for content: {}", content);
        }

        Ok(())
    }

    pub fn search(&mut self, query: &str, limit: usize) -> Result<Vec<(MemoryItem, f32)>> {
        let embeddings = self.model.embed(vec![query])?;
        let query_embedding = embeddings
            .first()
            .context("Failed to generate query embedding")?;

        let mut scores: Vec<(usize, f32)> = self
            .memories
            .iter()
            .enumerate()
            .map(|(idx, mem)| {
                let score = cosine_similarity(query_embedding, &mem.embedding);
                (idx, score)
            })
            .collect();

        // Sort by score descending
        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<(MemoryItem, f32)> = scores
            .into_iter()
            .take(limit)
            .map(|(idx, score)| (self.memories[idx].clone(), score))
            .collect();

        Ok(results)
    }
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let dot_product: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    dot_product / (norm_a * norm_b)
}
