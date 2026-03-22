mod catalog;
mod curate;
mod manifest;
mod materialize;

pub use catalog::{
    DatasetSplitSource, SciJudgeBenchSourceConfig, SciJudgePaperCandidate,
    load_scijudgebench_catalog,
};
pub use curate::{SamplingBucket, SamplingPolicy, build_curated_test_set};
pub use manifest::{CuratedPaperEntry, CuratedTestSet, load_test_set, save_test_set};
pub use materialize::{
    MaterializeOptions, MaterializeReport, MaterializedPaper, export_test_set, materialize_test_set,
};
