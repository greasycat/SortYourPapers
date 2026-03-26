use super::*;

pub(super) fn validate_saved_placement_progress(
    prepared_batches: &[PreparedPlacementBatch],
    snapshot: &OutputSnapshot,
    placement_mode: crate::papers::placement::PlacementMode,
    category_depth: u8,
    mut progress: PlacementBatchProgress,
) -> Result<PlacementBatchProgress> {
    if progress.completed_batches.is_empty() {
        return Ok(progress);
    }

    let expected_batches = prepared_batches
        .iter()
        .map(|batch| {
            (
                batch.batch_index,
                batch
                    .papers
                    .iter()
                    .map(|paper| paper.file_id.clone())
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<HashMap<_, _>>();

    for batch in &progress.completed_batches {
        let Some(expected_file_ids) = expected_batches.get(&batch.batch_index) else {
            return Err(AppError::Validation(format!(
                "saved placement batch {} no longer matches the current input",
                batch.batch_index
            )));
        };
        if &batch.file_ids != expected_file_ids {
            return Err(AppError::Validation(format!(
                "saved placement batch {} has inconsistent file ids",
                batch.batch_index
            )));
        }
        let batch_papers = prepared_batches
            .iter()
            .find(|prepared| prepared.batch_index == batch.batch_index)
            .map(|prepared| prepared.papers.as_slice())
            .ok_or_else(|| {
                AppError::Validation(format!(
                    "saved placement batch {} no longer matches the current input",
                    batch.batch_index
                ))
            })?;
        validate_placements(
            &batch.placements,
            batch_papers,
            snapshot,
            placement_mode,
            category_depth,
        )?;
    }

    progress
        .completed_batches
        .sort_by_key(|batch| batch.batch_index);
    Ok(progress)
}
