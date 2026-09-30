//! A simple container for problems found while checking the EPUB.
//!
//! Instead of stopping at the first problem, every check adds its findings
//! here so the user sees *all* the problems in a single run.

#[derive(Default)]
pub struct Report {
    /// Things that make the EPUB invalid. If there are any, no file is written.
    pub errors: Vec<String>,
    /// Things worth knowing about that don't make the EPUB invalid.
    pub warnings: Vec<String>,
}

impl Report {
    // `impl Into<String>` means "anything that can be turned into a String",
    // so callers can pass either a `&str` or a `String` (e.g. from `format!`).
    pub fn add_error(&mut self, message: impl Into<String>) {
        self.errors.push(message.into());
    }

    pub fn add_warning(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}
