use std::{env, fs, path::Path};

use crate::tui::forms::{
    ApiKeySourceMode, UiVerbosity, masked_value, parse_u8, parse_u64, parse_usize, provider_label,
    run_field_key, run_field_label,
};
use crate::{error::Result, llm::LlmProvider};

use super::{
    RunForm,
    state::{RunFormAnalysis, ValidationIssue, ValidationSeverity},
};

impl RunFormAnalysis {
    pub(crate) fn has_errors(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
    }

    pub(crate) fn field_issue(&self, field: usize) -> Option<&ValidationIssue> {
        self.issues
            .iter()
            .filter(|issue| issue.field == Some(field))
            .max_by_key(|issue| issue.severity.rank())
    }

    pub(crate) fn blocking_message(&self) -> String {
        let errors = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Error)
            .collect::<Vec<_>>();

        if errors.is_empty() {
            return "No blocking validation issues.".to_string();
        }

        let mut lines = vec!["Fix these issues before starting a run:".to_string()];
        for issue in errors.into_iter().take(4) {
            if let Some(field) = issue.field {
                lines.push(format!("- {}: {}", run_field_label(field), issue.message));
            } else {
                lines.push(format!("- {}", issue.message));
            }
        }
        lines.join("\n")
    }

    pub(crate) fn readiness_text(&self) -> String {
        let warnings = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Warning)
            .count();
        let infos = self
            .issues
            .iter()
            .filter(|issue| issue.severity == ValidationSeverity::Info)
            .count();

        if self.has_errors() {
            format!(
                "Fix {} blocking issue(s) before launch",
                self.issues
                    .iter()
                    .filter(|issue| issue.severity == ValidationSeverity::Error)
                    .count()
            )
        } else if warnings > 0 {
            format!("Ready to run with {warnings} warning(s) and {infos} note(s)")
        } else if infos > 0 {
            format!("Ready to run with {infos} note(s)")
        } else {
            "Ready to run".to_string()
        }
    }

    pub(crate) fn issue_counts(&self) -> (usize, usize, usize) {
        let mut infos = 0;
        let mut warnings = 0;
        let mut errors = 0;

        for issue in &self.issues {
            match issue.severity {
                ValidationSeverity::Info => infos += 1,
                ValidationSeverity::Warning => warnings += 1,
                ValidationSeverity::Error => errors += 1,
            }
        }

        (infos, warnings, errors)
    }
}

impl RunForm {
    pub(crate) fn analysis(&self) -> RunFormAnalysis {
        let mut issues = Vec::new();

        self.validate_required_directory(0, &self.input, true, &mut issues);
        self.validate_required_directory(1, &self.output, false, &mut issues);

        if self.llm_model.trim().is_empty() {
            issues.push(ValidationIssue {
                field: Some(14),
                severity: ValidationSeverity::Error,
                message: "Model is required.".to_string(),
            });
        }

        self.validate_numeric_field(3, &self.max_file_size_mb, parse_u64, &mut issues);
        self.validate_numeric_field(4, &self.page_cutoff, parse_u8, &mut issues);
        self.validate_numeric_field(5, &self.pdf_extract_workers, parse_usize, &mut issues);
        self.validate_numeric_field(6, &self.category_depth, parse_u8, &mut issues);
        self.validate_numeric_field(8, &self.taxonomy_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(9, &self.placement_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(18, &self.keyword_batch_size, parse_usize, &mut issues);
        self.validate_numeric_field(
            19,
            &self.subcategories_suggestion_number,
            parse_usize,
            &mut issues,
        );

        if matches!(self.llm_provider, LlmProvider::Openai | LlmProvider::Gemini)
            && self.api_key_value.trim().is_empty()
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Warning,
                message: format!(
                    "{} usually requires an API key unless credentials are supplied elsewhere.",
                    provider_label(self.llm_provider)
                ),
            });
        }

        if matches!(self.llm_provider, LlmProvider::Ollama) && !self.api_key_value.trim().is_empty()
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Info,
                message: "Ollama does not use the API key fields.".to_string(),
            });
        }

        if self.api_key_value.trim().is_empty()
            && matches!(
                self.api_key_source,
                ApiKeySourceMode::Command | ApiKeySourceMode::Env
            )
        {
            issues.push(ValidationIssue {
                field: Some(17),
                severity: ValidationSeverity::Warning,
                message: match self.api_key_source {
                    ApiKeySourceMode::Command => {
                        "Enter a shell command that prints the API key.".to_string()
                    }
                    ApiKeySourceMode::Env => {
                        "Enter the environment variable name that holds the API key.".to_string()
                    }
                    ApiKeySourceMode::Text => String::new(),
                },
            });
        }

        if self.api_key_source == ApiKeySourceMode::Env && !self.api_key_value.trim().is_empty() {
            match env::var(self.api_key_value.trim()) {
                Ok(value) if !value.trim().is_empty() => {}
                Ok(_) => issues.push(ValidationIssue {
                    field: Some(17),
                    severity: ValidationSeverity::Warning,
                    message: format!(
                        "Environment variable {} is set but empty.",
                        self.api_key_value.trim()
                    ),
                }),
                Err(_) => issues.push(ValidationIssue {
                    field: Some(17),
                    severity: ValidationSeverity::Warning,
                    message: format!(
                        "Environment variable {} is not set.",
                        self.api_key_value.trim()
                    ),
                }),
            }
        }

        if self.quiet && !matches!(self.verbosity, UiVerbosity::Normal) {
            issues.push(ValidationIssue {
                field: Some(21),
                severity: ValidationSeverity::Warning,
                message: "Quiet mode will suppress most of the extra output from verbose/debug."
                    .to_string(),
            });
        }

        if self.apply && self.rebuild {
            issues.push(ValidationIssue {
                field: Some(11),
                severity: ValidationSeverity::Warning,
                message: "Rebuild + apply will reclassify against a rebuilt taxonomy and then move files."
                    .to_string(),
            });
        }

        if self.use_current_folder_tree && self.rebuild {
            issues.push(ValidationIssue {
                field: Some(22),
                severity: ValidationSeverity::Info,
                message: "Rebuild ignores the current output tree, so this taxonomy hint will be inactive."
                    .to_string(),
            });
        }

        let config = if issues
            .iter()
            .any(|issue| issue.severity == ValidationSeverity::Error)
        {
            None
        } else {
            match self.build_config() {
                Ok(config) => Some(config),
                Err(err) => {
                    issues.push(ValidationIssue {
                        field: None,
                        severity: ValidationSeverity::Error,
                        message: err.to_string(),
                    });
                    None
                }
            }
        };

        RunFormAnalysis { config, issues }
    }

    fn validate_required_directory(
        &self,
        field: usize,
        value: &str,
        must_exist: bool,
        issues: &mut Vec<ValidationIssue>,
    ) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            issues.push(ValidationIssue {
                field: Some(field),
                severity: ValidationSeverity::Error,
                message: "Path is required.".to_string(),
            });
            return;
        }

        let path = Path::new(trimmed);
        match fs::metadata(path) {
            Ok(metadata) => {
                if !metadata.is_dir() {
                    issues.push(ValidationIssue {
                        field: Some(field),
                        severity: ValidationSeverity::Error,
                        message: "Path exists but is not a directory.".to_string(),
                    });
                }
            }
            Err(_) if must_exist => {
                issues.push(ValidationIssue {
                    field: Some(field),
                    severity: ValidationSeverity::Error,
                    message: "Folder does not exist.".to_string(),
                });
            }
            Err(_) => {
                issues.push(ValidationIssue {
                    field: Some(field),
                    severity: ValidationSeverity::Info,
                    message: "Folder does not exist yet. It will be created during apply mode."
                        .to_string(),
                });
            }
        }
    }

    fn validate_numeric_field<T>(
        &self,
        field: usize,
        value: &str,
        parse: impl Fn(&str, &str) -> Result<T>,
        issues: &mut Vec<ValidationIssue>,
    ) {
        if let Err(err) = parse(run_field_key(field), value) {
            issues.push(ValidationIssue {
                field: Some(field),
                severity: ValidationSeverity::Error,
                message: err.to_string(),
            });
        }
    }
}

pub(super) fn summarize_issue(issue: &ValidationIssue, _analysis: &RunFormAnalysis) -> String {
    if let Some(field) = issue.field {
        format!("{}: {}", run_field_label(field), issue.message)
    } else {
        issue.message.clone()
    }
}

pub(super) fn display_path_line(value: &str, default_value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default_value.to_string()
    } else {
        trimmed.to_string()
    }
}

pub(super) fn api_key_value_display(source: ApiKeySourceMode, value: &str) -> String {
    match source {
        ApiKeySourceMode::Text => masked_value(value),
        ApiKeySourceMode::Command | ApiKeySourceMode::Env => value.trim().to_string(),
    }
}

pub(super) fn api_key_summary(source: ApiKeySourceMode, value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "<empty>".to_string();
    }

    match source {
        ApiKeySourceMode::Text => format!("{} chars", trimmed.chars().count()),
        ApiKeySourceMode::Command => "shell command".to_string(),
        ApiKeySourceMode::Env => trimmed.to_string(),
    }
}
