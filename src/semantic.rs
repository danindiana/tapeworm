/// Cosine similarity search over in-memory embedding corpus.

/// Compute cosine similarity between two equal-length f32 vectors.
/// Returns a value in [-1.0, 1.0] where 1.0 = identical direction.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    debug_assert_eq!(a.len(), b.len(), "embedding dimension mismatch");
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let mag_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }
    (dot / (mag_a * mag_b)).clamp(-1.0, 1.0)
}

/// A loaded embedding entry from the database.
pub struct EmbeddingEntry {
    pub command_id: i64,
    pub embedding: Vec<f32>,
}

/// Find the top-k most similar commands to a query embedding.
/// Returns (command_id, similarity_score) sorted descending by score.
pub fn top_k_similar(
    query: &[f32],
    corpus: &[EmbeddingEntry],
    k: usize,
) -> Vec<(i64, f32)> {
    let mut scores: Vec<(i64, f32)> = corpus
        .iter()
        .filter(|e| e.embedding.len() == query.len())
        .map(|e| (e.command_id, cosine_similarity(query, &e.embedding)))
        .collect();

    // Partial sort: we only need the top k
    if scores.len() > k {
        scores.select_nth_unstable_by(k, |a, b| {
            b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)
        });
        scores.truncate(k);
    }
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors() {
        let v = vec![1.0f32, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn orthogonal_vectors() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![0.0f32, 1.0, 0.0];
        assert!(cosine_similarity(&a, &b).abs() < 1e-6);
    }

    #[test]
    fn opposite_vectors() {
        let a = vec![1.0f32, 2.0, 3.0];
        let b = vec![-1.0f32, -2.0, -3.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn top_k_ordering() {
        let query = vec![1.0f32, 0.0];
        let corpus = vec![
            EmbeddingEntry { command_id: 1, embedding: vec![0.9f32, 0.1] },  // ~high sim
            EmbeddingEntry { command_id: 2, embedding: vec![0.0f32, 1.0] },  // ~low sim
            EmbeddingEntry { command_id: 3, embedding: vec![1.0f32, 0.0] },  // exact match
        ];
        let results = top_k_similar(&query, &corpus, 2);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 3); // exact match first
        assert_eq!(results[1].0, 1); // high-sim second
    }

    #[test]
    fn dimension_mismatch_skipped() {
        let query = vec![1.0f32, 0.0];
        let corpus = vec![
            EmbeddingEntry { command_id: 1, embedding: vec![1.0f32, 0.0, 0.0] }, // wrong dim
            EmbeddingEntry { command_id: 2, embedding: vec![0.8f32, 0.2] },       // correct
        ];
        let results = top_k_similar(&query, &corpus, 5);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, 2);
    }
}
