use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// A fact is a piece of knowledge the agent has learned.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Fact {
    pub id: Uuid,
    pub category: String,
    pub key: String,
    pub value: String,
    /// Confidence score 0.0-1.0.
    pub confidence: f64,
    /// Where this fact came from.
    pub source: Option<String>,
    /// Embedding vector for similarity search (optional).
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Manages semantic memory — structured knowledge and facts.
pub struct SemanticMemory {
    /// In-memory fact index by category.
    facts: HashMap<String, Vec<Fact>>,
}

impl Default for SemanticMemory {
    fn default() -> Self {
        Self::new()
    }
}

impl SemanticMemory {
    pub fn new() -> Self {
        Self {
            facts: HashMap::new(),
        }
    }

    /// Store or update a fact.
    pub fn upsert(&mut self, fact: Fact) {
        let entry = self.facts.entry(fact.category.clone()).or_default();
        // Update existing or insert new
        if let Some(existing) = entry.iter_mut().find(|f| f.key == fact.key) {
            existing.value = fact.value;
            existing.confidence = fact.confidence;
            existing.updated_at = Utc::now();
            if fact.embedding.is_some() {
                existing.embedding = fact.embedding;
            }
        } else {
            entry.push(fact);
        }
    }

    /// Look up a specific fact.
    pub fn get(&self, category: &str, key: &str) -> Option<&Fact> {
        self.facts
            .get(category)
            .and_then(|facts| facts.iter().find(|f| f.key == key))
    }

    /// Remove a specific fact by category and key. Returns true if found and removed.
    pub fn remove(&mut self, category: &str, key: &str) -> bool {
        if let Some(facts) = self.facts.get_mut(category) {
            let before = facts.len();
            facts.retain(|f| f.key != key);
            let removed = facts.len() < before;
            // Clean up empty category
            if facts.is_empty() {
                self.facts.remove(category);
            }
            removed
        } else {
            false
        }
    }

    /// Remove all facts in a category. Returns the number of facts removed.
    pub fn remove_category(&mut self, category: &str) -> usize {
        self.facts.remove(category).map(|v| v.len()).unwrap_or(0)
    }

    /// Get all facts in a category.
    pub fn category(&self, category: &str) -> &[Fact] {
        self.facts
            .get(category)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Search across all facts using word-level matching.
    ///
    /// Matches when ANY query word appears in the category, key, or value.
    /// Results are scored by how many query words match, sorted best-first.
    pub fn search(&self, query: &str) -> Vec<&Fact> {
        let query_lower = query.to_lowercase();
        // Extract meaningful query words (skip very short words)
        let query_words: Vec<&str> = query_lower
            .split_whitespace()
            .filter(|w| w.len() >= 2)
            .collect();

        if query_words.is_empty() {
            // Fall back to full-string substring match
            return self
                .facts
                .values()
                .flat_map(|facts| {
                    facts.iter().filter(|f| {
                        f.key.to_lowercase().contains(&query_lower)
                            || f.value.to_lowercase().contains(&query_lower)
                    })
                })
                .collect();
        }

        // Score each fact by how many query words match in category+key+value
        let mut scored: Vec<(&Fact, usize)> = self
            .facts
            .iter()
            .flat_map(|(cat, facts)| {
                let cat_lower = cat.to_lowercase();
                let qw = &query_words;
                facts.iter().map(move |f| {
                    let key_lower = f.key.to_lowercase();
                    let val_lower = f.value.to_lowercase();
                    let hit_count = qw
                        .iter()
                        .filter(|w| {
                            cat_lower.contains(*w)
                                || key_lower.contains(*w)
                                || val_lower.contains(*w)
                        })
                        .count();
                    (f, hit_count)
                })
            })
            .filter(|(_, score)| *score > 0)
            .collect();

        // Sort by score descending
        scored.sort_by(|a, b| b.1.cmp(&a.1));
        scored.into_iter().map(|(f, _)| f).collect()
    }

    /// Cosine similarity search against stored embeddings.
    pub fn vector_search(&self, query_embedding: &[f32], top_k: usize) -> Vec<(&Fact, f32)> {
        let mut results: Vec<(&Fact, f32)> = self
            .facts
            .values()
            .flat_map(|facts| {
                facts.iter().filter_map(|f| {
                    f.embedding.as_ref().map(|emb| {
                        let similarity = cosine_similarity(query_embedding, emb);
                        (f, similarity)
                    })
                })
            })
            .collect();

        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(top_k);
        results
    }

    /// Get all categories.
    pub fn categories(&self) -> Vec<&str> {
        self.facts.keys().map(|s| s.as_str()).collect()
    }

    /// Get all facts across all categories.
    pub fn all_facts(&self) -> Vec<&Fact> {
        self.facts.values().flat_map(|v| v.iter()).collect()
    }

    /// Total number of facts stored.
    pub fn count(&self) -> usize {
        self.facts.values().map(|v| v.len()).sum()
    }
}

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}
