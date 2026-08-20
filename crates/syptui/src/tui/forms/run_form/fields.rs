//! Field table for the run form.
//!
//! Every run-form field is described exactly once here: its label, help text,
//! how it is edited, whether it belongs to the essential or advanced tier, and
//! where it sits in the three-column layout. Navigation, rendering, and
//! validation all read this table instead of carrying their own field lists.

/// How a field is edited from the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldKind {
    /// Free-form text entered through the edit overlay.
    Text,
    /// Boolean flipped in place.
    Toggle,
    /// Fixed set of values cycled in place.
    Choice,
    /// Launches the run instead of holding a value.
    Button,
}

/// Whether a field is shown by default or only in advanced mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FieldTier {
    /// Always visible.
    Essential,
    /// Visible only while the form is in advanced mode.
    Advanced,
}

/// Every field the run form can show, in no particular order.
///
/// Layout order comes from [`COLUMNS`], not from this declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunField {
    Input,
    Output,
    Recursive,
    MaxFileSizeMb,
    PageCutoff,
    PdfExtractWorkers,
    CategoryDepth,
    TaxonomyMode,
    TaxonomyBatchSize,
    UseCurrentFolderTree,
    KeywordBatchSize,
    SubcategoriesSuggestionNumber,
    PlacementMode,
    PlacementBatchSize,
    LlmProvider,
    LlmModel,
    LlmBaseUrl,
    ApiKeySource,
    ApiKeyValue,
    Apply,
    Rebuild,
    Verbosity,
    Quiet,
    RunButton,
}

/// A titled group of fields inside one layout column.
pub(crate) struct SectionSpec {
    pub(crate) title: &'static str,
    pub(crate) fields: &'static [RunField],
}

/// The run form layout: three columns of titled sections.
pub(crate) const COLUMNS: [&[SectionSpec]; 3] = [
    &[
        SectionSpec {
            title: "Paths & Scope",
            fields: &[RunField::Input, RunField::Output, RunField::Recursive],
        },
        SectionSpec {
            title: "Extraction",
            fields: &[
                RunField::MaxFileSizeMb,
                RunField::PageCutoff,
                RunField::PdfExtractWorkers,
            ],
        },
    ],
    &[
        SectionSpec {
            title: "Taxonomy",
            fields: &[
                RunField::CategoryDepth,
                RunField::TaxonomyMode,
                RunField::TaxonomyBatchSize,
                RunField::UseCurrentFolderTree,
                RunField::KeywordBatchSize,
                RunField::SubcategoriesSuggestionNumber,
            ],
        },
        SectionSpec {
            title: "Placement",
            fields: &[RunField::PlacementMode, RunField::PlacementBatchSize],
        },
    ],
    &[
        SectionSpec {
            title: "LLM & API",
            fields: &[
                RunField::LlmProvider,
                RunField::LlmModel,
                RunField::LlmBaseUrl,
                RunField::ApiKeySource,
                RunField::ApiKeyValue,
            ],
        },
        SectionSpec {
            title: "Run",
            fields: &[
                RunField::Apply,
                RunField::Rebuild,
                RunField::Verbosity,
                RunField::Quiet,
                RunField::RunButton,
            ],
        },
    ],
];

struct FieldDescriptor {
    key: &'static str,
    label: &'static str,
    help: &'static str,
    kind: FieldKind,
    tier: FieldTier,
}

impl RunField {
    /// Config key used in parse errors, matching the `AppConfig` field name.
    pub(crate) fn key(self) -> &'static str {
        self.descriptor().key
    }

    /// Short name shown in the form and in validation messages.
    pub(crate) fn label(self) -> &'static str {
        self.descriptor().label
    }

    /// Long description shown in the selected-field panel.
    pub(crate) fn help(self) -> &'static str {
        self.descriptor().help
    }

    pub(crate) fn kind(self) -> FieldKind {
        self.descriptor().kind
    }

    pub(crate) fn tier(self) -> FieldTier {
        self.descriptor().tier
    }

    /// Whether the field is shown for the current advanced-mode setting.
    pub(crate) fn visible(self, advanced: bool) -> bool {
        advanced || self.tier() == FieldTier::Essential
    }

    /// Whether the field opens the text edit overlay.
    pub(crate) fn editable(self) -> bool {
        self.kind() == FieldKind::Text
    }

    /// Whether the field holds a filesystem path, which uses the directory
    /// picker overlay instead of plain text entry.
    pub(crate) fn is_path(self) -> bool {
        matches!(self, Self::Input | Self::Output)
    }

    #[allow(clippy::too_many_lines)]
    fn descriptor(self) -> FieldDescriptor {
        match self {
            Self::Input => FieldDescriptor {
                key: "input",
                label: "Input Folder",
                help: "Source folder scanned for candidate PDFs. Must exist before launch.",
                kind: FieldKind::Text,
                tier: FieldTier::Essential,
            },
            Self::Output => FieldDescriptor {
                key: "output",
                label: "Output Folder",
                help: "Destination root for sorted papers. Created during apply if missing.",
                kind: FieldKind::Text,
                tier: FieldTier::Essential,
            },
            Self::Recursive => FieldDescriptor {
                key: "recursive",
                label: "Recursive Scan",
                help: "Scan nested folders inside the input path. Off means the top level only.",
                kind: FieldKind::Toggle,
                tier: FieldTier::Essential,
            },
            Self::MaxFileSizeMb => FieldDescriptor {
                key: "max_file_size_mb",
                label: "Max File Size (MB)",
                help: "Upper PDF size limit before extraction. Larger files are skipped.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::PageCutoff => FieldDescriptor {
                key: "page_cutoff",
                label: "Pages Per PDF",
                help: "Maximum pages extracted from each PDF. Keeps runs faster and cheaper.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::PdfExtractWorkers => FieldDescriptor {
                key: "pdf_extract_workers",
                label: "Extract Workers",
                help: "Parallel PDF extraction workers. Higher values trade more CPU for throughput.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::CategoryDepth => FieldDescriptor {
                key: "category_depth",
                label: "Category Depth",
                help: "Maximum taxonomy folder depth the run tries to build and place into.",
                kind: FieldKind::Text,
                tier: FieldTier::Essential,
            },
            Self::TaxonomyMode => FieldDescriptor {
                key: "taxonomy_mode",
                label: "Taxonomy Strategy",
                help: "How the taxonomy is synthesized from paper batches. Batch merge is the default.",
                kind: FieldKind::Choice,
                tier: FieldTier::Advanced,
            },
            Self::TaxonomyBatchSize => FieldDescriptor {
                key: "taxonomy_batch_size",
                label: "Taxonomy Batch Size",
                help: "Preliminary category groups sent in each taxonomy synthesis request.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::UseCurrentFolderTree => FieldDescriptor {
                key: "use_current_folder_tree",
                label: "Use Current Folder Tree",
                help: "Feed the existing output folder tree into taxonomy merge as optional naming guidance.",
                kind: FieldKind::Toggle,
                tier: FieldTier::Advanced,
            },
            Self::KeywordBatchSize => FieldDescriptor {
                key: "keyword_batch_size",
                label: "Keyword Batch Size",
                help: "Papers grouped into each keyword extraction request.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::SubcategoriesSuggestionNumber => FieldDescriptor {
                key: "subcategories_suggestion_number",
                label: "Target Subcategories",
                help: "Soft target for how many child categories a node should usually stay under.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::PlacementMode => FieldDescriptor {
                key: "placement_mode",
                label: "Placement Policy",
                help: "Whether placement must reuse existing folders or can introduce new ones.",
                kind: FieldKind::Choice,
                tier: FieldTier::Essential,
            },
            Self::PlacementBatchSize => FieldDescriptor {
                key: "placement_batch_size",
                label: "Placement Batch Size",
                help: "Papers classified together in each placement request.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::LlmProvider => FieldDescriptor {
                key: "llm_provider",
                label: "LLM Provider",
                help: "Model backend used for keywords, taxonomy synthesis, and placement.",
                kind: FieldKind::Choice,
                tier: FieldTier::Essential,
            },
            Self::LlmModel => FieldDescriptor {
                key: "llm_model",
                label: "Model",
                help: "Model name sent to the selected provider. Required.",
                kind: FieldKind::Text,
                tier: FieldTier::Essential,
            },
            Self::LlmBaseUrl => FieldDescriptor {
                key: "llm_base_url",
                label: "Base URL",
                help: "Custom provider endpoint. Leave blank to use the provider default.",
                kind: FieldKind::Text,
                tier: FieldTier::Advanced,
            },
            Self::ApiKeySource => FieldDescriptor {
                key: "api_key_source",
                label: "API Key Source",
                help: "How the API key is loaded: literal text, shell command output, or an environment variable.",
                kind: FieldKind::Choice,
                tier: FieldTier::Essential,
            },
            Self::ApiKeyValue => FieldDescriptor {
                key: "api_key_value",
                label: "API Key Value",
                help: "Used as the literal key, the shell command, or the environment variable name based on API Key Source.",
                kind: FieldKind::Text,
                tier: FieldTier::Essential,
            },
            Self::Apply => FieldDescriptor {
                key: "apply",
                label: "Apply Moves",
                help: "Execute file moves. Leave this off to keep the run in preview mode.",
                kind: FieldKind::Toggle,
                tier: FieldTier::Essential,
            },
            Self::Rebuild => FieldDescriptor {
                key: "rebuild",
                label: "Rebuild Output",
                help: "Ignore the current output tree and rebuild taxonomy before classifying.",
                kind: FieldKind::Toggle,
                tier: FieldTier::Advanced,
            },
            Self::Verbosity => FieldDescriptor {
                key: "verbosity",
                label: "Verbosity",
                help: "Backend log detail level: normal, verbose, or debug.",
                kind: FieldKind::Choice,
                tier: FieldTier::Advanced,
            },
            Self::Quiet => FieldDescriptor {
                key: "quiet",
                label: "Quiet Mode",
                help: "Reduce runtime output to warnings, errors, and essential summaries.",
                kind: FieldKind::Toggle,
                tier: FieldTier::Advanced,
            },
            Self::RunButton => FieldDescriptor {
                key: "run_button",
                label: "Run Button",
                help: "Launch the current run configuration. Equivalent to pressing r on the run form.",
                kind: FieldKind::Button,
                tier: FieldTier::Essential,
            },
        }
    }
}

/// Fields of one column that are visible for the current advanced setting, in
/// layout order.
pub(crate) fn visible_column_fields(column: usize, advanced: bool) -> Vec<RunField> {
    COLUMNS[column]
        .iter()
        .flat_map(|section| section.fields)
        .copied()
        .filter(|field| field.visible(advanced))
        .collect()
}

/// All visible fields, column by column, in layout order.
pub(crate) fn visible_fields(advanced: bool) -> Vec<RunField> {
    (0..COLUMNS.len())
        .flat_map(|column| visible_column_fields(column, advanced))
        .collect()
}
