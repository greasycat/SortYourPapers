use std::{
    io::{IsTerminal, stderr, stdout},
    time::Duration,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Verbosity {
    verbose: bool,
    debug: bool,
    quiet: bool,
}

impl Verbosity {
    pub fn new(verbose: bool, debug: bool, quiet: bool) -> Self {
        Self {
            verbose,
            debug,
            quiet,
        }
    }

    pub fn quiet(self) -> bool {
        self.quiet
    }

    pub fn silenced(self) -> Self {
        Self {
            verbose: self.verbose,
            debug: self.debug,
            quiet: true,
        }
    }

    pub fn verbose_enabled(self) -> bool {
        self.verbose && !self.quiet
    }

    pub fn debug_enabled(self) -> bool {
        self.debug && !self.quiet
    }

    pub fn use_color_stderr(self) -> bool {
        !self.quiet && stderr().is_terminal()
    }

    pub fn use_color_stdout(self) -> bool {
        !self.quiet && stdout().is_terminal()
    }

    pub fn info(self, message: impl AsRef<str>) {
        if !self.quiet {
            eprintln!("{}", message.as_ref());
        }
    }

    pub fn debug(self, message: impl AsRef<str>) {
        if self.verbose_enabled() {
            eprintln!("{}", message.as_ref());
        }
    }

    pub fn stage_line(self, stage: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag("STAGE", "1;36");
        let stage = self.paint(stage, "1;34");
        self.info(format!("{tag} {stage} {}", message.as_ref()));
    }

    pub fn run_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag(label, "1;35");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn success_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag(label, "1;32");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn warn_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag(label, "1;33");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn error_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag(label, "1;31");
        self.info(format!("{tag} {}", message.as_ref()));
    }

    pub fn debug_line(self, label: &str, message: impl AsRef<str>) {
        let tag = self.paint_tag(label, "1;90");
        self.debug(format!("{tag} {}", message.as_ref()));
    }

    pub fn header_stdout(self, title: &str) -> String {
        if self.use_color_stdout() {
            format!("\x1b[1;37;46m {title} \x1b[0m")
        } else {
            format!("== {title} ==")
        }
    }

    pub fn good(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;32")
    }

    pub fn warn(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;33")
    }

    pub fn bad(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;31")
    }

    pub fn accent(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "1;36")
    }

    pub fn muted(self, text: impl AsRef<str>) -> String {
        self.paint(text.as_ref(), "90")
    }

    fn paint_tag(self, text: &str, code: &str) -> String {
        self.paint(&format!("[{text}]"), code)
    }

    fn paint(self, text: &str, code: &str) -> String {
        if self.use_color_stderr() || self.use_color_stdout() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

pub fn format_duration(duration: Duration) -> String {
    if duration.as_secs_f64() >= 1.0 {
        format!("{:.3}s", duration.as_secs_f64())
    } else {
        format!("{:.1}ms", duration.as_secs_f64() * 1000.0)
    }
}
