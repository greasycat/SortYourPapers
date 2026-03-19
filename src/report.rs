use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::llm::LlmRunUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileAction {
    Move,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanAction {
    pub source: PathBuf,
    pub destination: PathBuf,
    pub action: FileAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunReport {
    pub scanned: usize,
    pub processed: usize,
    pub skipped: usize,
    pub failed: usize,
    pub actions: Vec<PlanAction>,
    pub dry_run: bool,
    #[serde(default)]
    pub llm_usage: LlmRunUsage,
}

impl RunReport {
    #[must_use]
    pub fn new(dry_run: bool) -> Self {
        Self {
            scanned: 0,
            processed: 0,
            skipped: 0,
            failed: 0,
            actions: Vec::new(),
            dry_run,
            llm_usage: LlmRunUsage::default(),
        }
    }
}

pub use crate::terminal::report::*;
