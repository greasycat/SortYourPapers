//! Measuring how good a run's organization actually is.

pub mod clustering;

pub use clustering::{ClusterAssignment, ClusteringMetrics, score};
