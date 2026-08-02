use crate::{ValidationError, ValidationErrorKind};
use std::collections::BTreeSet;

pub(crate) fn bounded_text(field: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::new(field, ValidationErrorKind::Empty));
    }
    if value.len() > max {
        return Err(ValidationError::new(
            field,
            ValidationErrorKind::TooLong {
                max,
                actual: value.len(),
            },
        ));
    }
    if value.chars().any(char::is_control) {
        return Err(ValidationError::new(
            field,
            ValidationErrorKind::InvalidFormat,
        ));
    }
    Ok(())
}

pub(crate) fn bounded_optional_text(
    field: &str,
    value: &Option<String>,
    max: usize,
) -> Result<(), ValidationError> {
    if let Some(value) = value {
        bounded_text(field, value, max)?;
    }
    Ok(())
}

pub(crate) fn bounded_count(field: &str, actual: usize, max: usize) -> Result<(), ValidationError> {
    if actual > max {
        return Err(ValidationError::new(
            field,
            ValidationErrorKind::TooMany { max, actual },
        ));
    }
    Ok(())
}

pub(crate) fn unique_text<'a>(
    field: &str,
    values: impl IntoIterator<Item = &'a str>,
) -> Result<(), ValidationError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(ValidationError::new(
                field,
                ValidationErrorKind::InvalidFormat,
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_slug(field: &str, value: &str, max: usize) -> Result<(), ValidationError> {
    bounded_text(field, value, max)?;
    let mut previous_separator = false;
    for (index, byte) in value.bytes().enumerate() {
        let separator = matches!(byte, b'_' | b'-');
        if !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || separator)
            || (separator && (index == 0 || index + 1 == value.len() || previous_separator))
        {
            return Err(ValidationError::new(
                field,
                ValidationErrorKind::InvalidFormat,
            ));
        }
        previous_separator = separator;
    }
    Ok(())
}
