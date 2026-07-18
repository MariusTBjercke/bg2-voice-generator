//! Path sanitizers shared by profile ZIP import (zip-slip protection).

use crate::error::AppError;

/// Join an UNTRUSTED relative path onto `base`, refusing anything that could escape it.
pub fn safe_rel_join(
    base: &std::path::Path,
    untrusted: &str,
) -> Result<std::path::PathBuf, AppError> {
    use std::path::Component;

    let normalized = untrusted.replace('\\', "/");
    let mut clean = std::path::PathBuf::new();
    for comp in std::path::Path::new(&normalized).components() {
        match comp {
            Component::Normal(c) => clean.push(c),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(AppError::Other(format!(
                    "unsafe path {untrusted:?} in imported bundle \
                     (absolute paths and `..` are not allowed)"
                )));
            }
        }
    }
    if clean.as_os_str().is_empty() {
        return Err(AppError::Other(format!(
            "unsafe path {untrusted:?} in imported bundle (empty after sanitizing)"
        )));
    }
    Ok(base.join(clean))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_rel_join_accepts_plain_relative_paths() {
        let base = std::path::Path::new("C:/base");
        for ok in ["clip.wav", "IMOEN/clip.wav", "./a/b.wav", "a\\b.wav"] {
            let joined = safe_rel_join(base, ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
            assert!(joined.starts_with(base), "{ok} -> {joined:?}");
        }
    }

    #[test]
    fn safe_rel_join_rejects_traversal() {
        let base = std::path::Path::new("C:/base");
        for bad in ["../x", "a/../../b", "/abs", "C:/abs"] {
            assert!(safe_rel_join(base, bad).is_err(), "{bad}");
        }
    }
}
