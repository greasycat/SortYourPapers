use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use syp_core::error::{AppError, Result};

use crate::{CuratedPaperEntry, CuratedTestSet, SciJudgePaperCandidate};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SamplingBucket {
    Top,
    Bottom,
    Random,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SamplingPolicy {
    pub top_n_per_category: usize,
    pub bottom_n_per_category: usize,
    pub random_n_per_category: usize,
    pub random_seed: u64,
    pub per_subcategory_cap: usize,
}

impl Default for SamplingPolicy {
    fn default() -> Self {
        Self {
            top_n_per_category: 5,
            bottom_n_per_category: 5,
            random_n_per_category: 5,
            random_seed: 0x5A17_DA7A,
            per_subcategory_cap: 2,
        }
    }
}

pub fn build_curated_test_set(
    candidates: &[SciJudgePaperCandidate],
    policy: &SamplingPolicy,
) -> Result<CuratedTestSet> {
    validate_policy(policy)?;

    let mut groups = BTreeMap::<String, Vec<&SciJudgePaperCandidate>>::new();
    for candidate in candidates {
        if candidate.arxiv_id.trim().is_empty() {
            continue;
        }
        let category = if candidate.category.trim().is_empty() {
            "uncategorized".to_string()
        } else {
            candidate.category.trim().to_string()
        };
        groups.entry(category).or_default().push(candidate);
    }

    if groups.is_empty() {
        return Err(AppError::Validation(
            "no candidates with usable arxiv ids were available for curation".to_string(),
        ));
    }

    let mut selected = Vec::<CuratedPaperEntry>::new();
    let mut seen_ids = HashSet::<String>::new();
    for (_category, papers) in groups {
        let mut ordered = papers;
        ordered.sort_by(|left, right| {
            right
                .citations
                .cmp(&left.citations)
                .then_with(|| left.arxiv_id.cmp(&right.arxiv_id))
        });

        let top = pick_ranked(
            &ordered,
            SamplingBucket::Top,
            policy.top_n_per_category,
            policy.per_subcategory_cap,
            &mut seen_ids,
        );
        let bottom = {
            let mut ascending = ordered.clone();
            ascending.reverse();
            pick_ranked(
                &ascending,
                SamplingBucket::Bottom,
                policy.bottom_n_per_category,
                policy.per_subcategory_cap,
                &mut seen_ids,
            )
        };
        let random = pick_random(
            &ordered,
            policy.random_n_per_category,
            policy.per_subcategory_cap,
            policy.random_seed,
            &mut seen_ids,
        );

        for entry in top.into_iter().chain(bottom).chain(random) {
            selected.push(entry);
        }
    }

    selected.sort_by(|left, right| {
        left.category
            .cmp(&right.category)
            .then_with(|| {
                left.selection_bucket
                    .sort_order()
                    .cmp(&right.selection_bucket.sort_order())
            })
            .then_with(|| right.citations.cmp(&left.citations))
            .then_with(|| left.paper_id.cmp(&right.paper_id))
    });

    Ok(CuratedTestSet {
        id: "scijudgebench-diverse".to_string(),
        description: "Curated SciJudgeBench arXiv test set with top, bottom, and deterministic random citation samples per category.".to_string(),
        source_dataset: "OpenMOSS-Team/SciJudgeBench".to_string(),
        selection_policy: policy.clone(),
        generated_at_ms: now_unix_ms()?,
        papers: selected,
    })
}

fn validate_policy(policy: &SamplingPolicy) -> Result<()> {
    if policy.top_n_per_category == 0
        && policy.bottom_n_per_category == 0
        && policy.random_n_per_category == 0
    {
        return Err(AppError::Validation(
            "sampling policy must request at least one paper per category".to_string(),
        ));
    }
    if policy.per_subcategory_cap == 0 {
        return Err(AppError::Validation(
            "per_subcategory_cap must be greater than zero".to_string(),
        ));
    }
    Ok(())
}

fn pick_ranked(
    ordered: &[&SciJudgePaperCandidate],
    bucket: SamplingBucket,
    limit: usize,
    per_subcategory_cap: usize,
    seen_ids: &mut HashSet<String>,
) -> Vec<CuratedPaperEntry> {
    if limit == 0 {
        return Vec::new();
    }

    pick_candidates(
        ordered.iter().copied(),
        bucket,
        limit,
        per_subcategory_cap,
        seen_ids,
    )
}

fn pick_random(
    ordered: &[&SciJudgePaperCandidate],
    limit: usize,
    per_subcategory_cap: usize,
    random_seed: u64,
    seen_ids: &mut HashSet<String>,
) -> Vec<CuratedPaperEntry> {
    if limit == 0 {
        return Vec::new();
    }

    let mut shuffled = ordered.to_vec();
    shuffled.sort_by_key(|candidate| random_order_key(candidate, random_seed));
    pick_candidates(
        shuffled.into_iter(),
        SamplingBucket::Random,
        limit,
        per_subcategory_cap,
        seen_ids,
    )
}

fn pick_candidates<'a>(
    ordered: impl IntoIterator<Item = &'a SciJudgePaperCandidate>,
    bucket: SamplingBucket,
    limit: usize,
    per_subcategory_cap: usize,
    seen_ids: &mut HashSet<String>,
) -> Vec<CuratedPaperEntry> {
    let mut picked = Vec::new();
    let mut overflow = Vec::new();
    let mut subcategory_counts = HashMap::<String, usize>::new();

    for candidate in ordered {
        if seen_ids.contains(candidate.arxiv_id.as_str()) {
            continue;
        }

        let subcategory = normalized_subcategory(candidate);
        let count = subcategory_counts.get(&subcategory).copied().unwrap_or(0);
        if count >= per_subcategory_cap {
            overflow.push(candidate);
            continue;
        }

        seen_ids.insert(candidate.arxiv_id.clone());
        *subcategory_counts.entry(subcategory).or_default() += 1;
        picked.push(to_manifest_entry(candidate, bucket));
        if picked.len() == limit {
            return picked;
        }
    }

    for candidate in overflow {
        if seen_ids.contains(candidate.arxiv_id.as_str()) {
            continue;
        }
        seen_ids.insert(candidate.arxiv_id.clone());
        picked.push(to_manifest_entry(candidate, bucket));
        if picked.len() == limit {
            break;
        }
    }

    picked
}

fn to_manifest_entry(
    candidate: &SciJudgePaperCandidate,
    bucket: SamplingBucket,
) -> CuratedPaperEntry {
    CuratedPaperEntry {
        paper_id: paper_id_from_arxiv_id(&candidate.arxiv_id),
        arxiv_id: candidate.arxiv_id.clone(),
        canonical_pdf_url: arxiv_pdf_url(&candidate.arxiv_id),
        title: candidate.title.clone(),
        category: if candidate.category.trim().is_empty() {
            "uncategorized".to_string()
        } else {
            candidate.category.trim().to_string()
        },
        subcategory: normalized_subcategory(candidate),
        citations: candidate.citations,
        date: candidate.date.clone(),
        abstract_excerpt: excerpt(&candidate.abstract_text, 320),
        selection_bucket: bucket,
        sha256: None,
        byte_size: None,
    }
}

fn normalized_subcategory(candidate: &SciJudgePaperCandidate) -> String {
    if candidate.subcategory.trim().is_empty() {
        "uncategorized".to_string()
    } else {
        candidate.subcategory.trim().to_string()
    }
}

fn paper_id_from_arxiv_id(arxiv_id: &str) -> String {
    format!("arxiv-{}", arxiv_id.replace('/', "-"))
}

fn arxiv_pdf_url(arxiv_id: &str) -> String {
    format!(
        "https://arxiv.org/pdf/{}.pdf",
        arxiv_id.trim().trim_end_matches(".pdf")
    )
}

fn excerpt(value: &str, limit: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= limit {
        return trimmed.to_string();
    }

    let mut out = String::new();
    for ch in trimmed.chars().take(limit.saturating_sub(1)) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn random_order_key(candidate: &SciJudgePaperCandidate, seed: u64) -> (u64, &str) {
    let mut hasher = DefaultHasher::new();
    seed.hash(&mut hasher);
    candidate.category.hash(&mut hasher);
    candidate.arxiv_id.hash(&mut hasher);
    (hasher.finish(), candidate.arxiv_id.as_str())
}

fn now_unix_ms() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| AppError::Execution(format!("system clock error: {err}")))?;
    i64::try_from(elapsed.as_millis())
        .map_err(|_| AppError::Execution("timestamp exceeded i64 range".to_string()))
}

impl SamplingBucket {
    fn sort_order(self) -> u8 {
        match self {
            Self::Top => 0,
            Self::Bottom => 1,
            Self::Random => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SamplingBucket, SamplingPolicy, build_curated_test_set};
    use crate::SciJudgePaperCandidate;

    #[test]
    fn sampling_builds_top_bottom_and_random_buckets() {
        let candidates = vec![
            sample_candidate("1001.0001", "CS", "cs.AI", 100),
            sample_candidate("1001.0002", "CS", "cs.AI", 50),
            sample_candidate("1001.0003", "CS", "cs.LG", 20),
            sample_candidate("1001.0004", "CS", "cs.CL", 10),
            sample_candidate("1001.0005", "CS", "cs.IR", 1),
        ];
        let policy = SamplingPolicy {
            top_n_per_category: 1,
            bottom_n_per_category: 1,
            random_n_per_category: 1,
            random_seed: 7,
            per_subcategory_cap: 1,
        };

        let set = build_curated_test_set(&candidates, &policy).expect("build test set");

        assert_eq!(set.papers.len(), 3);
        assert_eq!(set.papers[0].selection_bucket, SamplingBucket::Top);
        assert_eq!(set.papers[1].selection_bucket, SamplingBucket::Bottom);
        assert_eq!(set.papers[2].selection_bucket, SamplingBucket::Random);
    }

    #[test]
    fn subcategory_cap_spreads_ranked_selection() {
        let candidates = vec![
            sample_candidate("1001.0001", "Physics", "astro-ph", 100),
            sample_candidate("1001.0002", "Physics", "astro-ph", 90),
            sample_candidate("1001.0003", "Physics", "hep-th", 80),
        ];
        let policy = SamplingPolicy {
            top_n_per_category: 2,
            bottom_n_per_category: 0,
            random_n_per_category: 0,
            random_seed: 1,
            per_subcategory_cap: 1,
        };

        let set = build_curated_test_set(&candidates, &policy).expect("build test set");

        assert_eq!(set.papers.len(), 2);
        assert_eq!(set.papers[0].subcategory, "astro-ph");
        assert_eq!(set.papers[1].subcategory, "hep-th");
    }

    fn sample_candidate(
        arxiv_id: &str,
        category: &str,
        subcategory: &str,
        citations: u64,
    ) -> SciJudgePaperCandidate {
        SciJudgePaperCandidate {
            arxiv_id: arxiv_id.to_string(),
            title: format!("Paper {arxiv_id}"),
            abstract_text: "Abstract".to_string(),
            category: category.to_string(),
            subcategory: subcategory.to_string(),
            citations,
            date: Some("2024-01-01".to_string()),
            source_splits: vec!["train".to_string()],
        }
    }
}
