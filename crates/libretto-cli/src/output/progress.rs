//! Progress bar and spinner utilities.

use indicatif::{
    MultiProgress as IndicatifMultiProgress, ProgressBar as IndicatifProgressBar,
    ProgressStyle as IndicatifProgressStyle,
};
use std::time::Duration;

/// Progress bar style presets
#[derive(Debug, Clone, Copy)]
pub enum ProgressStyle {
    /// Standard progress bar with percentage
    Bar,
    /// Download progress with bytes
    Download,
    /// Spinner for indeterminate progress
    Spinner,
    /// Package installation progress
    Install,
    /// Extraction progress
    Extract,
    /// Resolution progress
    Resolve,
}

impl ProgressStyle {
    /// Get the indicatif template for this style
    const fn template(&self, unicode: bool) -> &'static str {
        match self {
            Self::Bar if unicode => {
                "{spinner:.green} [{bar:40.cyan/blue}] {pos}/{len} ({percent}%)"
            }
            Self::Bar => "{spinner} [{bar:40}] {pos}/{len} ({percent}%)",
            Self::Download if unicode => {
                "{spinner:.green} {msg:.cyan} [{bar:30.green/dim}] {bytes}/{total_bytes} ({bytes_per_sec})"
            }
            Self::Download => "{spinner} {msg} [{bar:30}] {bytes}/{total_bytes} ({bytes_per_sec})",
            Self::Spinner if unicode => "{spinner:.green} {msg}",
            Self::Spinner => "{spinner} {msg}",
            Self::Install if unicode => {
                "{spinner:.green} Installing {msg:.cyan} [{bar:25.green/dim}] {pos}/{len}"
            }
            Self::Install => "{spinner} Installing {msg} [{bar:25}] {pos}/{len}",
            Self::Extract if unicode => "{spinner:.green} Extracting {msg:.cyan}",
            Self::Extract => "{spinner} Extracting {msg}",
            Self::Resolve if unicode => "{spinner:.green} Resolving dependencies... {msg}",
            Self::Resolve => "{spinner} Resolving dependencies... {msg}",
        }
    }

    /// Get spinner characters
    const fn spinner_chars(&self, unicode: bool) -> &'static str {
        if unicode {
            "⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"
        } else {
            "-\\|/"
        }
    }

    /// Convert to indicatif style
    pub fn to_indicatif(self, unicode: bool) -> IndicatifProgressStyle {
        IndicatifProgressStyle::default_bar()
            .template(self.template(unicode))
            .expect("valid template")
            .tick_chars(self.spinner_chars(unicode))
            .progress_chars(if unicode { "█▓▒░" } else { "=>-" })
    }
}

/// Wrapper around indicatif `ProgressBar` with our styling
pub struct ProgressBar {
    inner: IndicatifProgressBar,
}

impl ProgressBar {
    /// Create a new progress bar with the given length
    pub fn new(len: u64, style: ProgressStyle) -> Self {
        let unicode = crate::output::unicode_enabled();
        let pb = IndicatifProgressBar::new(len);
        pb.set_style(style.to_indicatif(unicode));
        pb.enable_steady_tick(Duration::from_millis(80));
        Self { inner: pb }
    }

    /// Create a hidden progress bar (for quiet mode)
    pub fn hidden() -> Self {
        Self {
            inner: IndicatifProgressBar::hidden(),
        }
    }

    /// Create a progress bar from indicatif (for `MultiProgress`)
    pub fn from_indicatif(pb: IndicatifProgressBar, style: ProgressStyle) -> Self {
        let unicode = crate::output::unicode_enabled();
        pb.set_style(style.to_indicatif(unicode));
        pb.enable_steady_tick(Duration::from_millis(80));
        Self { inner: pb }
    }

    /// Set the current position
    pub fn set_position(&self, pos: u64) {
        self.inner.set_position(pos);
    }

    /// Increment the position
    pub fn inc(&self, delta: u64) {
        self.inner.inc(delta);
    }

    /// Set the message
    pub fn set_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.inner.set_message(msg);
    }

    /// Set the length
    pub fn set_length(&self, len: u64) {
        self.inner.set_length(len);
    }

    /// Finish the progress bar
    pub fn finish(&self) {
        self.inner.finish();
    }

    /// Finish with a message
    pub fn finish_with_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.inner.finish_with_message(msg);
    }

    /// Finish and clear the progress bar
    pub fn finish_and_clear(&self) {
        self.inner.finish_and_clear();
    }

    /// Abandon the progress bar (finish without clearing)
    pub fn abandon(&self) {
        self.inner.abandon();
    }

    /// Abandon with a message
    pub fn abandon_with_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.inner.abandon_with_message(msg);
    }

    /// Get the inner indicatif progress bar
    pub const fn inner(&self) -> &IndicatifProgressBar {
        &self.inner
    }
}

/// Spinner for indeterminate progress
pub struct Spinner {
    inner: IndicatifProgressBar,
}

impl Spinner {
    /// Create a new spinner with a message
    pub fn new(msg: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        let unicode = crate::output::unicode_enabled();
        let pb = IndicatifProgressBar::new_spinner();
        pb.set_style(ProgressStyle::Spinner.to_indicatif(unicode));
        pb.set_message(msg);
        pb.enable_steady_tick(Duration::from_millis(80));
        Self { inner: pb }
    }

    /// Create a hidden spinner
    pub fn hidden() -> Self {
        Self {
            inner: IndicatifProgressBar::hidden(),
        }
    }

    /// Set the message
    pub fn set_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.inner.set_message(msg);
    }

    /// Finish the spinner
    pub fn finish(&self) {
        self.inner.finish();
    }

    /// Finish with a message
    pub fn finish_with_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.inner.finish_with_message(msg);
    }

    /// Finish and clear the spinner
    pub fn finish_and_clear(&self) {
        self.inner.finish_and_clear();
    }
}

/// Multi-progress container for parallel operations
pub struct MultiProgress {
    inner: IndicatifMultiProgress,
}

impl MultiProgress {
    /// Create a new multi-progress container
    pub fn new() -> Self {
        Self {
            inner: IndicatifMultiProgress::new(),
        }
    }

    /// Create a hidden multi-progress (for quiet mode)
    pub fn hidden() -> Self {
        Self {
            inner: IndicatifMultiProgress::with_draw_target(indicatif::ProgressDrawTarget::hidden()),
        }
    }

    /// Add a new progress bar
    pub fn add(&self, len: u64, style: ProgressStyle) -> ProgressBar {
        let pb = self.inner.add(IndicatifProgressBar::new(len));
        ProgressBar::from_indicatif(pb, style)
    }

    /// Add a new spinner
    pub fn add_spinner(&self, msg: impl Into<std::borrow::Cow<'static, str>>) -> Spinner {
        let unicode = crate::output::unicode_enabled();
        let pb = self.inner.add(IndicatifProgressBar::new_spinner());
        pb.set_style(ProgressStyle::Spinner.to_indicatif(unicode));
        pb.set_message(msg);
        pb.enable_steady_tick(Duration::from_millis(80));
        Spinner { inner: pb }
    }

    /// Remove a progress bar
    pub fn remove(&self, pb: &ProgressBar) {
        self.inner.remove(pb.inner());
    }

    /// Clear all progress bars
    pub fn clear(&self) -> std::io::Result<()> {
        self.inner.clear()
    }

    /// Get the inner indicatif multi-progress
    pub const fn inner(&self) -> &IndicatifMultiProgress {
        &self.inner
    }
}

impl Default for MultiProgress {
    fn default() -> Self {
        Self::new()
    }
}

/// Create a quick spinner that auto-clears on drop
pub struct AutoSpinner {
    spinner: Spinner,
    success_msg: Option<String>,
}

impl AutoSpinner {
    /// Create a new auto-clearing spinner
    pub fn new(msg: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        Self {
            spinner: Spinner::new(msg),
            success_msg: None,
        }
    }

    /// Set the message to show on success
    pub fn success_message(mut self, msg: impl Into<String>) -> Self {
        self.success_msg = Some(msg.into());
        self
    }

    /// Update the spinner message
    pub fn set_message(&self, msg: impl Into<std::borrow::Cow<'static, str>>) {
        self.spinner.set_message(msg);
    }

    /// Finish successfully
    pub fn finish(mut self) {
        if let Some(msg) = self.success_msg.take() {
            self.spinner.finish_with_message(msg);
        } else {
            self.spinner.finish_and_clear();
        }
        std::mem::forget(self); // Don't run drop
    }
}

impl Drop for AutoSpinner {
    fn drop(&mut self) {
        self.spinner.finish_and_clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_style_template() {
        // Just ensure templates are valid
        for style in [
            ProgressStyle::Bar,
            ProgressStyle::Download,
            ProgressStyle::Spinner,
            ProgressStyle::Install,
        ] {
            let _ = style.to_indicatif(true);
            let _ = style.to_indicatif(false);
        }
    }
}
