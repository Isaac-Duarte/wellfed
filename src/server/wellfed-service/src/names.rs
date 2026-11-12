//! The one check every name and label the server keeps passes: trimmed,
//! then 1 to [`NAME_MAX`] characters.

use crate::Invalid;

/// The most characters a name or label holds, once trimmed.
pub(crate) const NAME_MAX: usize = 64;

/// `text` trimmed, when it holds 1 to [`NAME_MAX`] characters; else
/// `invalid`.
pub(crate) fn bounded(text: &str, invalid: Invalid) -> Result<&str, Invalid> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > NAME_MAX {
        return Err(invalid);
    }
    Ok(text)
}
