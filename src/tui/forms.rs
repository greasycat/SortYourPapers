mod extract_form;
mod run_form;

use crate::{
    error::{AppError, Result},
    llm::LlmProvider,
    papers::extract::ExtractorMode,
    placement::PlacementMode,
    taxonomy::TaxonomyMode,
};

pub(super) use self::{extract_form::ExtractForm, run_form::RunForm};

pub(super) const HOME_ITEMS: [&str; 5] = [
    "Run Papers",
    "Sessions",
    "Extract Text",
    "Init Config",
    "Quit",
];

pub(super) const RUN_FIELD_LABELS: [&str; 21] = [
    "input",
    "output",
    "recursive",
    "max_file_size_mb",
    "page_cutoff",
    "pdf_extract_workers",
    "category_depth",
    "taxonomy_mode",
    "taxonomy_batch_size",
    "placement_batch_size",
    "placement_mode",
    "rebuild",
    "apply",
    "llm_provider",
    "llm_model",
    "llm_base_url",
    "api_key",
    "keyword_batch_size",
    "subcategories_suggestion_number",
    "verbosity",
    "quiet",
];

pub(super) const EXTRACT_FIELD_LABELS: [&str; 5] = [
    "files",
    "page_cutoff",
    "extractor",
    "pdf_extract_workers",
    "verbosity",
];

#[derive(Debug, Clone, Copy)]
pub(super) enum UiVerbosity {
    Normal,
    Verbose,
    Debug,
}

impl UiVerbosity {
    pub(super) fn count(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Verbose => 1,
            Self::Debug => 2,
        }
    }

    pub(super) fn next(self) -> Self {
        match self {
            Self::Normal => Self::Verbose,
            Self::Verbose => Self::Debug,
            Self::Debug => Self::Normal,
        }
    }

    pub(super) fn previous(self) -> Self {
        match self {
            Self::Normal => Self::Debug,
            Self::Verbose => Self::Normal,
            Self::Debug => Self::Verbose,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Verbose => "verbose",
            Self::Debug => "debug",
        }
    }
}

fn empty_string_to_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn bool_label(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn masked_value(value: &str) -> String {
    if value.is_empty() {
        String::new()
    } else {
        "*".repeat(value.len().min(8))
    }
}

fn provider_label(value: LlmProvider) -> &'static str {
    match value {
        LlmProvider::Openai => "openai",
        LlmProvider::Ollama => "ollama",
        LlmProvider::Gemini => "gemini",
    }
}

fn taxonomy_mode_label(value: TaxonomyMode) -> &'static str {
    match value {
        TaxonomyMode::Global => "global",
        TaxonomyMode::BatchMerge => "batch-merge",
    }
}

fn placement_mode_label(value: PlacementMode) -> &'static str {
    match value {
        PlacementMode::ExistingOnly => "existing-only",
        PlacementMode::AllowNew => "allow-new",
    }
}

fn extractor_label(value: ExtractorMode) -> &'static str {
    match value {
        ExtractorMode::Auto => "auto",
        ExtractorMode::PdfOxide => "pdf-oxide",
        ExtractorMode::Pdftotext => "pdftotext",
    }
}

fn cycle_provider(value: LlmProvider, direction: i8) -> LlmProvider {
    let all = [
        LlmProvider::Openai,
        LlmProvider::Ollama,
        LlmProvider::Gemini,
    ];
    cycle_enum(value, &all, direction)
}

fn cycle_taxonomy_mode(value: TaxonomyMode, direction: i8) -> TaxonomyMode {
    let all = [TaxonomyMode::Global, TaxonomyMode::BatchMerge];
    cycle_enum(value, &all, direction)
}

fn cycle_placement_mode(value: PlacementMode, direction: i8) -> PlacementMode {
    let all = [PlacementMode::ExistingOnly, PlacementMode::AllowNew];
    cycle_enum(value, &all, direction)
}

fn cycle_extractor(value: ExtractorMode, direction: i8) -> ExtractorMode {
    let all = [
        ExtractorMode::Auto,
        ExtractorMode::PdfOxide,
        ExtractorMode::Pdftotext,
    ];
    cycle_enum(value, &all, direction)
}

fn cycle_enum<T>(value: T, values: &[T], direction: i8) -> T
where
    T: Copy + PartialEq,
{
    let index = values
        .iter()
        .position(|candidate| *candidate == value)
        .unwrap_or(0);
    let next = if direction >= 0 {
        (index + 1) % values.len()
    } else if index == 0 {
        values.len() - 1
    } else {
        index - 1
    };
    values[next]
}

fn parse_u64(name: &str, value: &str) -> Result<u64> {
    value
        .trim()
        .parse::<u64>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}

fn parse_usize(name: &str, value: &str) -> Result<usize> {
    value
        .trim()
        .parse::<usize>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}

fn parse_u8(name: &str, value: &str) -> Result<u8> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|err| AppError::Validation(format!("invalid {name}: {err}")))
}
