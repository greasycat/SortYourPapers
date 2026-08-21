//! Measuring how good a run's organization actually is.

pub mod bench;
pub mod calibration;
pub mod clustering;

pub use bench::{BenchDocument, StrategyResult, run_bench};
pub use calibration::{CalibrationReport, calibrate};
pub use clustering::{ClusterAssignment, ClusteringMetrics, score};
