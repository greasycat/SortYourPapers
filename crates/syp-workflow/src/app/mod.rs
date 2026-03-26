mod debug_seed;
mod extract_text;
mod path_resolution;
mod reference_index;
mod run;

pub use debug_seed::run_debug_tui;
#[cfg(test)]
pub(crate) use debug_seed::{
    apply_debug_taxonomy_suggestion, seed_debug_stages, simulate_debug_taxonomy_review,
};
pub use extract_text::run_extract_text;
pub use reference_index::index_reference_manifest;
pub use run::{run, run_with_args};

#[cfg(test)]
mod tests;
