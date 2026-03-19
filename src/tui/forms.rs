mod extract_form;
mod run_form;

use crate::{
    error::{AppError, Result},
    llm::LlmProvider,
    papers::extract::ExtractorMode,
    papers::placement::PlacementMode,
    papers::taxonomy::TaxonomyMode,
};

pub(super) use self::extract_form::ExtractForm;
pub(super) use self::run_form::RunForm;
#[cfg(test)]
pub(super) use self::run_form::ValidationSeverity;

struct RunFieldDescriptor {
    key: &'static str,
    label: &'static str,
    help: &'static str,
}

const RUN_FIELDS: [RunFieldDescriptor; 21] = [
    RunFieldDescriptor {
        key: "input",
        label: "Input Folder",
        help: "Folder to scan for PDF files. This path must exist and be a directory before a run can start.",
    },
    RunFieldDescriptor {
        key: "output",
        label: "Output Folder",
        help: "Destination root for sorted papers. A missing folder is allowed and will be created during apply-mode moves.",
    },
    RunFieldDescriptor {
        key: "recursive",
        label: "Recursive Scan",
        help: "Include nested folders under the input directory when discovering candidate PDFs.",
    },
    RunFieldDescriptor {
        key: "max_file_size_mb",
        label: "Max File Size (MB)",
        help: "Skip PDFs larger than this limit before extraction. Must be greater than zero.",
    },
    RunFieldDescriptor {
        key: "page_cutoff",
        label: "Pages Per PDF",
        help: "Maximum number of pages to extract from each PDF. Must be greater than zero.",
    },
    RunFieldDescriptor {
        key: "pdf_extract_workers",
        label: "Extract Workers",
        help: "Number of parallel PDF extraction workers. Higher values increase concurrency but may use more CPU.",
    },
    RunFieldDescriptor {
        key: "category_depth",
        label: "Category Depth",
        help: "Target folder depth used for preliminary categories and the synthesized taxonomy.",
    },
    RunFieldDescriptor {
        key: "taxonomy_mode",
        label: "Taxonomy Strategy",
        help: "Choose how taxonomy synthesis is framed. Batch merge is the current default workflow.",
    },
    RunFieldDescriptor {
        key: "taxonomy_batch_size",
        label: "Taxonomy Batch Size",
        help: "Number of aggregated preliminary-category entries processed per taxonomy synthesis batch.",
    },
    RunFieldDescriptor {
        key: "placement_batch_size",
        label: "Placement Batch Size",
        help: "Number of papers classified in each placement request.",
    },
    RunFieldDescriptor {
        key: "placement_mode",
        label: "Placement Policy",
        help: "Choose whether placements must use existing folders only or may propose new folders.",
    },
    RunFieldDescriptor {
        key: "rebuild",
        label: "Rebuild Output",
        help: "Ignore the current output tree and reclassify against a rebuilt taxonomy instead of existing folders.",
    },
    RunFieldDescriptor {
        key: "apply",
        label: "Apply Moves",
        help: "Switch from preview mode to real file moves. Leave this off to inspect the plan without changing files.",
    },
    RunFieldDescriptor {
        key: "llm_provider",
        label: "LLM Provider",
        help: "Backend used for keyword extraction, taxonomy synthesis, and placement decisions.",
    },
    RunFieldDescriptor {
        key: "llm_model",
        label: "Model",
        help: "Model identifier sent to the selected provider. This field should not be blank.",
    },
    RunFieldDescriptor {
        key: "llm_base_url",
        label: "Base URL",
        help: "Optional custom endpoint for provider requests. Leave blank to use the provider default.",
    },
    RunFieldDescriptor {
        key: "api_key",
        label: "API Key",
        help: "Optional provider credential. Gemini and OpenAI typically require this unless credentials are supplied elsewhere.",
    },
    RunFieldDescriptor {
        key: "keyword_batch_size",
        label: "Keyword Batch Size",
        help: "Number of papers included in each keyword extraction batch.",
    },
    RunFieldDescriptor {
        key: "subcategories_suggestion_number",
        label: "Target Subcategories",
        help: "Prompt guidance for how many subcategories each taxonomy node should usually stay under.",
    },
    RunFieldDescriptor {
        key: "verbosity",
        label: "Verbosity",
        help: "Controls how much runtime detail the CLI backend emits: normal, verbose, or debug.",
    },
    RunFieldDescriptor {
        key: "quiet",
        label: "Quiet Mode",
        help: "Suppress most progress and summary output while still showing warnings and errors.",
    },
];

pub(super) const EXTRACT_FIELD_LABELS: [&str; 5] = [
    "PDF Files",
    "Pages Per PDF",
    "Extractor",
    "Extract Workers",
    "Verbosity",
];

pub(super) fn run_field_key(index: usize) -> &'static str {
    RUN_FIELDS[index].key
}

pub(super) fn run_field_label(index: usize) -> &'static str {
    RUN_FIELDS[index].label
}

pub(super) fn run_field_help(index: usize) -> &'static str {
    RUN_FIELDS[index].help
}

pub(super) fn extract_field_label(index: usize) -> &'static str {
    EXTRACT_FIELD_LABELS[index]
}

#[derive(Debug, Clone, Copy)]
pub(super) enum UiVerbosity {
    Normal,
    Verbose,
    Debug,
}

impl UiVerbosity {
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

    pub(super) fn raw(self) -> u8 {
        match self {
            Self::Normal => 0,
            Self::Verbose => 1,
            Self::Debug => 2,
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

fn extractor_label(value: ExtractorMode) -> &'static str {
    match value {
        ExtractorMode::Auto => "auto",
        ExtractorMode::PdfOxide => "pdf-oxide",
        ExtractorMode::Pdftotext => "pdftotext",
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

fn cycle_provider(value: LlmProvider, direction: i8) -> LlmProvider {
    let all = [
        LlmProvider::Openai,
        LlmProvider::Ollama,
        LlmProvider::Gemini,
    ];
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

fn cycle_taxonomy_mode(value: TaxonomyMode, direction: i8) -> TaxonomyMode {
    let all = [TaxonomyMode::Global, TaxonomyMode::BatchMerge];
    cycle_enum(value, &all, direction)
}

fn cycle_placement_mode(value: PlacementMode, direction: i8) -> PlacementMode {
    let all = [PlacementMode::ExistingOnly, PlacementMode::AllowNew];
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
