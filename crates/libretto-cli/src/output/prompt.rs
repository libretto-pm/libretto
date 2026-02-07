//! Interactive prompt utilities.

use dialoguer::{
    Confirm as DialoguerConfirm, Input as DialoguerInput, MultiSelect as DialoguerMultiSelect,
    Select as DialoguerSelect, theme::ColorfulTheme,
};
use std::io::{self, IsTerminal};

/// Check if interactive prompts are available
pub fn is_interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Get the dialoguer theme based on color settings
fn get_theme() -> ColorfulTheme {
    ColorfulTheme::default()
}

type InputValidator = dyn Fn(&String) -> Result<(), String>;

/// Confirmation prompt
pub struct Confirm {
    message: String,
    default: Option<bool>,
}

impl Confirm {
    /// Create a new confirmation prompt
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            default: None,
        }
    }

    /// Set the default value
    pub const fn default(mut self, value: bool) -> Self {
        self.default = Some(value);
        self
    }

    /// Show the prompt and get the result
    pub fn prompt(self) -> io::Result<bool> {
        if !is_interactive() {
            return Ok(self.default.unwrap_or(false));
        }

        let theme = get_theme();
        let mut prompt = DialoguerConfirm::with_theme(&theme).with_prompt(&self.message);

        if let Some(default) = self.default {
            prompt = prompt.default(default);
        }

        prompt.interact().map_err(io::Error::other)
    }
}

/// Text input prompt
pub struct Input {
    message: String,
    default: Option<String>,
    allow_empty: bool,
    validator: Option<Box<InputValidator>>,
}

impl Input {
    /// Create a new input prompt
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            default: None,
            allow_empty: false,
            validator: None,
        }
    }

    /// Set the default value
    pub fn default(mut self, value: impl Into<String>) -> Self {
        self.default = Some(value.into());
        self
    }

    /// Allow empty input
    pub const fn allow_empty(mut self, allow: bool) -> Self {
        self.allow_empty = allow;
        self
    }

    /// Add a validator
    pub fn validate<F>(mut self, validator: F) -> Self
    where
        F: Fn(&String) -> Result<(), String> + 'static,
    {
        self.validator = Some(Box::new(validator));
        self
    }

    /// Show the prompt and get the result
    pub fn prompt(self) -> io::Result<String> {
        if !is_interactive() {
            return Ok(self.default.unwrap_or_default());
        }

        let theme = get_theme();
        let mut prompt = DialoguerInput::<String>::with_theme(&theme)
            .with_prompt(&self.message)
            .allow_empty(self.allow_empty);

        if let Some(default) = &self.default {
            prompt = prompt.default(default.clone());
        }

        if let Some(validator) = self.validator {
            prompt = prompt.validate_with(move |input: &String| validator(input));
        }

        prompt.interact_text().map_err(io::Error::other)
    }
}

/// Selection prompt
pub struct Select<T> {
    message: String,
    items: Vec<T>,
    default: Option<usize>,
}

impl<T: std::fmt::Display> Select<T> {
    /// Create a new selection prompt
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            items: Vec::new(),
            default: None,
        }
    }

    /// Add items to select from
    pub fn items(mut self, items: impl IntoIterator<Item = T>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    /// Set the default selection index
    pub const fn default(mut self, index: usize) -> Self {
        self.default = Some(index);
        self
    }

    /// Show the prompt and get the selected index
    pub fn prompt(&self) -> io::Result<usize> {
        if !is_interactive() {
            return Ok(self.default.unwrap_or(0));
        }

        let theme = get_theme();
        let mut prompt = DialoguerSelect::with_theme(&theme)
            .with_prompt(&self.message)
            .items(&self.items);

        if let Some(default) = self.default {
            prompt = prompt.default(default);
        }

        prompt.interact().map_err(io::Error::other)
    }

    /// Show the prompt and get the selected item
    pub fn prompt_item(self) -> io::Result<T>
    where
        T: Clone,
    {
        let index = self.prompt()?;
        self.items
            .get(index)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid selection"))
    }
}

/// Multi-selection prompt
pub struct MultiSelect<T> {
    message: String,
    items: Vec<T>,
    defaults: Vec<bool>,
}

impl<T: std::fmt::Display> MultiSelect<T> {
    /// Create a new multi-selection prompt
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            items: Vec::new(),
            defaults: Vec::new(),
        }
    }

    /// Add items to select from
    pub fn items(mut self, items: impl IntoIterator<Item = T>) -> Self {
        self.items = items.into_iter().collect();
        self
    }

    /// Set which items are selected by default
    pub fn defaults(mut self, defaults: impl IntoIterator<Item = bool>) -> Self {
        self.defaults = defaults.into_iter().collect();
        self
    }

    /// Show the prompt and get the selected indices
    pub fn prompt(&self) -> io::Result<Vec<usize>> {
        if !is_interactive() {
            return Ok(self
                .defaults
                .iter()
                .enumerate()
                .filter_map(|(i, &selected)| if selected { Some(i) } else { None })
                .collect());
        }

        let theme = get_theme();
        let mut prompt = DialoguerMultiSelect::with_theme(&theme)
            .with_prompt(&self.message)
            .items(&self.items);

        if !self.defaults.is_empty() {
            prompt = prompt.defaults(&self.defaults);
        }

        prompt.interact().map_err(io::Error::other)
    }

    /// Show the prompt and get the selected items
    pub fn prompt_items(self) -> io::Result<Vec<T>>
    where
        T: Clone,
    {
        let indices = self.prompt()?;
        Ok(indices
            .into_iter()
            .filter_map(|i| self.items.get(i).cloned())
            .collect())
    }
}

impl<T: std::fmt::Display + Clone> MultiSelect<T> {
    /// Show the prompt and get the selected items (clone version)
    pub fn prompt_items_cloned(self) -> io::Result<Vec<T>> {
        let indices = self.prompt()?;
        Ok(indices
            .into_iter()
            .filter_map(|i| self.items.get(i).cloned())
            .collect())
    }
}

/// Password input prompt
pub struct Password {
    message: String,
    confirmation: Option<String>,
}

impl Password {
    /// Create a new password prompt
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            confirmation: None,
        }
    }

    /// Require password confirmation
    pub fn with_confirmation(mut self, message: impl Into<String>) -> Self {
        self.confirmation = Some(message.into());
        self
    }

    /// Show the prompt and get the password
    pub fn prompt(self) -> io::Result<String> {
        if !is_interactive() {
            return Err(io::Error::other(
                "cannot prompt for password in non-interactive mode",
            ));
        }

        let theme = get_theme();
        let mut prompt = dialoguer::Password::with_theme(&theme).with_prompt(&self.message);

        if let Some(confirmation) = &self.confirmation {
            prompt = prompt.with_confirmation(confirmation, "Passwords do not match");
        }

        prompt.interact().map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_interactive() {
        // This will vary based on test environment
        let _ = is_interactive();
    }
}
