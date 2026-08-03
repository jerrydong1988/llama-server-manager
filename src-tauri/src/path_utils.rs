use std::path::Path;

fn normalize_path_text(raw: &str, windows_semantics: bool) -> String {
    let mut value = if windows_semantics {
        raw.trim().replace('\\', "/")
    } else {
        raw.trim().to_string()
    };
    if windows_semantics {
        let lower = value.to_ascii_lowercase();
        if lower.starts_with("//?/unc/") {
            value = format!("//{}", &value[8..]);
        } else if lower.starts_with("//?/") {
            value = value[4..].to_string();
        }
    }

    let is_unc = windows_semantics && value.starts_with("//");
    let is_drive_rooted = windows_semantics
        && value.as_bytes().get(1) == Some(&b':')
        && value.as_bytes().get(2) == Some(&b'/');
    let is_posix_rooted = !is_unc && !is_drive_rooted && value.starts_with('/');

    let (prefix, body, protected_segments) = if is_unc {
        ("//".to_string(), &value[2..], 2usize)
    } else if is_drive_rooted {
        (value[..3].to_string(), &value[3..], 0usize)
    } else if is_posix_rooted {
        ("/".to_string(), &value[1..], 0usize)
    } else {
        (String::new(), value.as_str(), 0usize)
    };

    let mut segments: Vec<&str> = Vec::new();
    for segment in body.split('/') {
        if segment.is_empty() || segment == "." {
            continue;
        }
        if segment == ".." {
            if segments.len() > protected_segments && segments.last() != Some(&"..") {
                segments.pop();
            } else if prefix.is_empty() {
                segments.push(segment);
            }
            continue;
        }
        segments.push(segment);
    }

    let joined = segments.join("/");
    let normalized = if joined.is_empty() {
        prefix
    } else if prefix.is_empty() || prefix.ends_with('/') {
        format!("{prefix}{joined}")
    } else {
        format!("{prefix}/{joined}")
    };

    let normalized = if normalized.is_empty() && !raw.trim().is_empty() {
        ".".to_string()
    } else {
        normalized
    };

    if windows_semantics {
        normalized.to_lowercase()
    } else {
        normalized
    }
}

pub(crate) fn path_identity_key(path: &Path) -> String {
    normalize_path_text(&path.to_string_lossy(), cfg!(windows))
}

pub(crate) fn paths_equal(left: &Path, right: &Path) -> bool {
    path_identity_key(left) == path_identity_key(right)
}

fn path_is_within_text(candidate: &str, root: &str, windows_semantics: bool) -> bool {
    let candidate = normalize_path_text(candidate, windows_semantics);
    let root = normalize_path_text(root, windows_semantics);
    if root.is_empty() {
        return false;
    }
    if root == "." {
        return candidate == "."
            || !(candidate.is_empty()
                || candidate == ".."
                || candidate.starts_with("../")
                || candidate.starts_with('/')
                || (candidate.as_bytes().get(1) == Some(&b':')
                    && candidate.as_bytes().get(2) == Some(&b'/')));
    }
    if candidate == root {
        return true;
    }
    if root.ends_with('/') {
        candidate.starts_with(&root)
    } else {
        candidate.starts_with(&format!("{root}/"))
    }
}

pub(crate) fn path_is_within(candidate: &Path, root: &Path) -> bool {
    path_is_within_text(
        &candidate.to_string_lossy(),
        &root.to_string_lossy(),
        cfg!(windows),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_drive_aliases_share_one_identity() {
        assert_eq!(
            normalize_path_text(r"\\?\C:\Models\Qwen", true),
            "c:/models/qwen"
        );
        assert_eq!(
            normalize_path_text(r"c:/models/./Llama/../Qwen/", true),
            "c:/models/qwen"
        );
    }

    #[test]
    fn windows_unc_aliases_share_one_identity() {
        assert_eq!(
            normalize_path_text(r"\\?\UNC\Server\Share\Models", true),
            "//server/share/models"
        );
        assert_eq!(
            normalize_path_text(r"//server/share/models/", true),
            "//server/share/models"
        );
    }

    #[test]
    fn containment_observes_directory_boundaries() {
        assert!(path_is_within_text(
            r"\\?\C:\Models\Qwen\model.gguf",
            "c:/models",
            true
        ));
        assert!(!path_is_within_text(
            r"C:\Models-Old\model.gguf",
            r"\\?\c:\models",
            true
        ));
        assert!(path_is_within_text(
            r"\\?\UNC\SERVER\Share\Models\model.gguf",
            r"\\server\share\models",
            true
        ));
    }

    #[test]
    fn unc_parent_segments_cannot_escape_share_root() {
        assert_eq!(
            normalize_path_text(r"\\server\share\..\models", true),
            "//server/share/models"
        );
    }

    #[test]
    fn posix_paths_remain_case_sensitive() {
        assert_ne!(
            normalize_path_text("/Models/Qwen", false),
            normalize_path_text("/models/qwen", false)
        );
        assert_ne!(
            normalize_path_text(r"/models/name\with-backslash", false),
            normalize_path_text("/models/name/with-backslash", false)
        );
    }

    #[test]
    fn current_directory_contains_safe_relative_children_only() {
        assert!(path_is_within_text("./weights/model.gguf", ".", false));
        assert!(!path_is_within_text("../outside/model.gguf", ".", false));
        assert!(!path_is_within_text("/outside/model.gguf", ".", false));
    }
}
