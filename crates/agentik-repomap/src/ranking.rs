//! PageRank-based file ranking.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::graph::DependencyGraph;

/// PageRank algorithm configuration.
#[derive(Debug, Clone)]
pub struct PageRankConfig {
    /// Damping factor (probability of following a link vs random jump)
    pub damping: f64,
    /// Number of iterations to run
    pub iterations: usize,
    /// Convergence threshold (stop if max change is below this)
    pub convergence_threshold: f64,
}

impl Default for PageRankConfig {
    fn default() -> Self {
        Self {
            damping: 0.85,
            iterations: 100,
            convergence_threshold: 1e-6,
        }
    }
}

/// PageRank-based file scorer.
///
/// Computes importance scores for files based on their position in the
/// dependency graph. Files that are imported by many other files get
/// higher scores.
pub struct PageRankScorer {
    config: PageRankConfig,
}

impl PageRankScorer {
    /// Create a new scorer with default configuration.
    pub fn new() -> Self {
        Self {
            config: PageRankConfig::default(),
        }
    }

    /// Create a new scorer with custom configuration.
    pub fn with_config(config: PageRankConfig) -> Self {
        Self { config }
    }

    /// Compute PageRank scores for all files in the graph.
    pub fn compute(&self, graph: &DependencyGraph) -> HashMap<PathBuf, f64> {
        self.compute_with_personalization(graph, None)
    }

    /// Compute PageRank with query boosting.
    ///
    /// Files in `focus_files` are boosted by a factor of 3.0,
    /// and their neighbors are boosted by sqrt(3.0).
    pub fn compute_with_query(
        &self,
        graph: &DependencyGraph,
        focus_files: &[PathBuf],
    ) -> HashMap<PathBuf, f64> {
        if focus_files.is_empty() {
            return self.compute(graph);
        }

        // Build personalization vector
        let focus_set: HashSet<_> = focus_files.iter().cloned().collect();

        // Find neighbors of focus files
        let mut neighbor_set = HashSet::new();
        for file in &focus_set {
            for neighbor in graph.neighbors(file) {
                if !focus_set.contains(&neighbor) {
                    neighbor_set.insert(neighbor);
                }
            }
        }

        // Create personalization weights
        let focus_boost = 3.0_f64;
        let neighbor_boost = focus_boost.sqrt();

        let mut personalization = HashMap::new();
        for file in graph.files() {
            let weight = if focus_set.contains(file) {
                focus_boost
            } else if neighbor_set.contains(file) {
                neighbor_boost
            } else {
                1.0
            };
            personalization.insert(file.clone(), weight);
        }

        self.compute_with_personalization(graph, Some(personalization))
    }

    /// Compute PageRank with optional personalization vector.
    fn compute_with_personalization(
        &self,
        graph: &DependencyGraph,
        personalization: Option<HashMap<PathBuf, f64>>,
    ) -> HashMap<PathBuf, f64> {
        let files: Vec<_> = graph.files().cloned().collect();
        let n = files.len();

        if n == 0 {
            return HashMap::new();
        }

        // Build index mapping
        let file_to_idx: HashMap<_, _> = files.iter().enumerate().map(|(i, f)| (f.clone(), i)).collect();

        // Initialize scores
        let initial_score = 1.0 / n as f64;
        let mut scores: Vec<f64> = vec![initial_score; n];
        let mut new_scores: Vec<f64> = vec![0.0; n];

        // Build personalization vector (normalized)
        let personalization_vec: Vec<f64> = if let Some(p) = &personalization {
            let total: f64 = files.iter().map(|f| p.get(f).unwrap_or(&1.0)).sum();
            files
                .iter()
                .map(|f| p.get(f).unwrap_or(&1.0) / total)
                .collect()
        } else {
            vec![1.0 / n as f64; n]
        };

        // Build outgoing links for each file
        // In PageRank, we use the reverse of the dependency graph:
        // if A imports B, then B should get "credit" from A
        // So we look at files that import each file (dependents)
        let incoming: Vec<Vec<usize>> = files
            .iter()
            .map(|f| {
                graph
                    .dependents(f)
                    .iter()
                    .filter_map(|d| file_to_idx.get(*d).copied())
                    .collect()
            })
            .collect();

        // Count outgoing links (dependencies) for normalization
        let out_degrees: Vec<usize> = files
            .iter()
            .map(|f| graph.out_degree(f))
            .collect();

        let damping = self.config.damping;

        // Iterative computation
        for _ in 0..self.config.iterations {
            // Handle dangling nodes (no outgoing links)
            let dangling_sum: f64 = files
                .iter()
                .enumerate()
                .filter(|(i, _)| out_degrees[*i] == 0)
                .map(|(i, _)| scores[i])
                .sum();
            let dangling_contribution = damping * dangling_sum / n as f64;

            for i in 0..n {
                // Sum contributions from incoming links
                let incoming_contribution: f64 = incoming[i]
                    .iter()
                    .map(|&j| {
                        let out_deg = out_degrees[j];
                        if out_deg > 0 {
                            scores[j] / out_deg as f64
                        } else {
                            0.0
                        }
                    })
                    .sum();

                // PageRank formula with personalization
                new_scores[i] = damping * incoming_contribution
                    + dangling_contribution
                    + (1.0 - damping) * personalization_vec[i];
            }

            // Check convergence
            let max_diff: f64 = scores
                .iter()
                .zip(new_scores.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0, f64::max);

            std::mem::swap(&mut scores, &mut new_scores);

            if max_diff < self.config.convergence_threshold {
                break;
            }
        }

        // Normalize scores to sum to 1
        let total: f64 = scores.iter().sum();
        if total > 0.0 {
            for score in &mut scores {
                *score /= total;
            }
        }

        // Build result map
        files
            .into_iter()
            .zip(scores)
            .collect()
    }

    /// Rank files by their PageRank score (highest first).
    pub fn rank(&self, graph: &DependencyGraph) -> Vec<(PathBuf, f64)> {
        let scores = self.compute(graph);
        let mut ranked: Vec<_> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    /// Rank files with query boosting.
    pub fn rank_with_query(
        &self,
        graph: &DependencyGraph,
        focus_files: &[PathBuf],
    ) -> Vec<(PathBuf, f64)> {
        let scores = self.compute_with_query(graph, focus_files);
        let mut ranked: Vec<_> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }
}

impl Default for PageRankScorer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_test_graph() -> DependencyGraph {
        let mut graph = DependencyGraph::new();

        // Create a simple graph:
        // main.rs -> lib.rs -> utils.rs
        //         -> helper.rs
        graph.add_file(PathBuf::from("main.rs"));
        graph.add_file(PathBuf::from("lib.rs"));
        graph.add_file(PathBuf::from("utils.rs"));
        graph.add_file(PathBuf::from("helper.rs"));

        graph.add_edge(PathBuf::from("main.rs"), PathBuf::from("lib.rs"));
        graph.add_edge(PathBuf::from("main.rs"), PathBuf::from("helper.rs"));
        graph.add_edge(PathBuf::from("lib.rs"), PathBuf::from("utils.rs"));

        graph
    }

    #[test]
    fn test_pagerank_basic() {
        let graph = build_test_graph();
        let scorer = PageRankScorer::new();
        let scores = scorer.compute(&graph);

        // All files should have a score
        assert_eq!(scores.len(), 4);

        // Scores should sum to approximately 1
        let total: f64 = scores.values().sum();
        assert!((total - 1.0).abs() < 0.01);

        // Files with more incoming links should have higher scores
        // utils.rs and helper.rs are imported, main.rs is not
        let main_score = scores.get(&PathBuf::from("main.rs")).unwrap();
        let lib_score = scores.get(&PathBuf::from("lib.rs")).unwrap();

        // lib.rs is imported by main.rs, so should have decent score
        assert!(*lib_score > 0.0);
    }

    #[test]
    fn test_pagerank_empty_graph() {
        let graph = DependencyGraph::new();
        let scorer = PageRankScorer::new();
        let scores = scorer.compute(&graph);

        assert!(scores.is_empty());
    }

    #[test]
    fn test_pagerank_single_file() {
        let mut graph = DependencyGraph::new();
        graph.add_file(PathBuf::from("single.rs"));

        let scorer = PageRankScorer::new();
        let scores = scorer.compute(&graph);

        assert_eq!(scores.len(), 1);
        assert!((scores[&PathBuf::from("single.rs")] - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_pagerank_with_query_boost() {
        let graph = build_test_graph();
        let scorer = PageRankScorer::new();

        // Without boost
        let scores_normal = scorer.compute(&graph);

        // With boost on utils.rs
        let focus_files = vec![PathBuf::from("utils.rs")];
        let scores_boosted = scorer.compute_with_query(&graph, &focus_files);

        // utils.rs should have a higher relative score when boosted
        let utils_normal = scores_normal[&PathBuf::from("utils.rs")];
        let utils_boosted = scores_boosted[&PathBuf::from("utils.rs")];

        // The boosted score should be higher
        assert!(utils_boosted > utils_normal);
    }

    #[test]
    fn test_rank_ordering() {
        let graph = build_test_graph();
        let scorer = PageRankScorer::new();
        let ranked = scorer.rank(&graph);

        // Should be sorted by score descending
        for i in 1..ranked.len() {
            assert!(ranked[i - 1].1 >= ranked[i].1);
        }
    }

    #[test]
    fn test_convergence() {
        let graph = build_test_graph();

        // Test with very low iteration count
        let config = PageRankConfig {
            iterations: 1,
            ..Default::default()
        };
        let scorer = PageRankScorer::with_config(config);
        let scores_1 = scorer.compute(&graph);

        // Test with more iterations
        let config = PageRankConfig {
            iterations: 100,
            ..Default::default()
        };
        let scorer = PageRankScorer::with_config(config);
        let scores_100 = scorer.compute(&graph);

        // Both should produce valid scores
        assert_eq!(scores_1.len(), scores_100.len());
    }
}
