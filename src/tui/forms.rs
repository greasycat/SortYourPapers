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
#[cfg(test)]
pub(super) use self::run_form::ValidationSeverity;
pub(super) use self::run_form::{RunForm, list_relative_directories};

struct RunFieldDescriptor {
    key: &'static str,
    label: &'static str,
    help: &'static str,
}

const RUN_FIELDS: [RunFieldDescriptor; 22] = [
    RunFieldDescriptor {
        key: "input",
        label: "Input Folder",
        help: "Source folder scanned for candidate PDFs. Must exist before launch.",
    },
    RunFieldDescriptor {
        key: "output",
        label: "Output Folder",
        help: "Destination root for sorted papers. Created during apply if missing.",
    },
    RunFieldDescriptor {
        key: "recursive",
        label: "Recursive Scan",
        help: "Scan nested folders inside the input path. Off means the top level only.",
    },
    RunFieldDescriptor {
        key: "max_file_size_mb",
        label: "Max File Size (MB)",
        help: "Upper PDF size limit before extraction. Larger files are skipped.",
    },
    RunFieldDescriptor {
        key: "page_cutoff",
        label: "Pages Per PDF",
        help: "Maximum pages extracted from each PDF. Keeps runs faster and cheaper.",
    },
    RunFieldDescriptor {
        key: "pdf_extract_workers",
        label: "Extract Workers",
        help: "Parallel PDF extraction workers. Higher values trade more CPU for throughput.",
    },
    RunFieldDescriptor {
        key: "category_depth",
        label: "Category Depth",
        help: "Maximum taxonomy folder depth the run tries to build and place into.",
    },
    RunFieldDescriptor {
        key: "taxonomy_mode",
        label: "Taxonomy Strategy",
        help: "How the taxonomy is synthesized from paper batches. Batch merge is the default.",
    },
    RunFieldDescriptor {
        key: "taxonomy_batch_size",
        label: "Taxonomy Batch Size",
        help: "Preliminary category groups sent in each taxonomy synthesis request.",
    },
    RunFieldDescriptor {
        key: "placement_batch_size",
        label: "Placement Batch Size",
        help: "Papers classified together in each placement request.",
    },
    RunFieldDescriptor {
        key: "placement_mode",
        label: "Placement Policy",
        help: "Whether placement must reuse existing folders or can introduce new ones.",
    },
    RunFieldDescriptor {
        key: "rebuild",
        label: "Rebuild Output",
        help: "Ignore the current output tree and rebuild taxonomy before classifying.",
    },
    RunFieldDescriptor {
        key: "apply",
        label: "Apply Moves",
        help: "Execute file moves. Leave this off to keep the run in preview mode.",
    },
    RunFieldDescriptor {
        key: "llm_provider",
        label: "LLM Provider",
        help: "Model backend used for keywords, taxonomy synthesis, and placement.",
    },
    RunFieldDescriptor {
        key: "llm_model",
        label: "Model",
        help: "Model name sent to the selected provider. Required.",
    },
    RunFieldDescriptor {
        key: "llm_base_url",
        label: "Base URL",
        help: "Custom provider endpoint. Leave blank to use the provider default.",
    },
    RunFieldDescriptor {
        key: "api_key_source",
        label: "API Key Source",
        help: "How the API key is loaded: literal text, shell command output, or an environment variable.",
    },
    RunFieldDescriptor {
        key: "api_key_value",
        label: "API Key Value",
        help: "Used as the literal key, the shell command, or the environment variable name based on API Key Source.",
    },
    RunFieldDescriptor {
        key: "keyword_batch_size",
        label: "Keyword Batch Size",
        help: "Papers grouped into each keyword extraction request.",
    },
    RunFieldDescriptor {
        key: "subcategories_suggestion_number",
        label: "Target Subcategories",
        help: "Soft target for how many child categories a node should usually stay under.",
    },
    RunFieldDescriptor {
        key: "verbosity",
        label: "Verbosity",
        help: "Backend log detail level: normal, verbose, or debug.",
    },
    RunFieldDescriptor {
        key: "quiet",
        label: "Quiet Mode",
        help: "Reduce runtime output to warnings, errors, and essential summaries.",
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApiKeySourceMode {
    Text,
    Command,
    Env,
}

impl ApiKeySourceMode {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Text => Self::Command,
            Self::Command => Self::Env,
            Self::Env => Self::Text,
        }
    }

    pub(super) fn previous(self) -> Self {
        match self {
            Self::Text => Self::Env,
            Self::Command => Self::Text,
            Self::Env => Self::Command,
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Command => "command",
            Self::Env => "env",
        }
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
