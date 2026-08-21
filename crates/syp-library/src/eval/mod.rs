//! Measuring how good a run's organization actually is.

pub mod calibration;
pub mod clustering;

pub use calibration::{CalibrationReport, calibrate};
pub use clustering::{ClusterAssignment, ClusteringMetrics, score};
