use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModelShardDescriptor {
    pub base: String,
    pub index: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModelArtifactError {
    Unavailable,
    Incomplete,
}

pub(crate) fn parse_model_shard_name(name: &str) -> Option<ModelShardDescriptor> {
    let lowercase = name.to_ascii_lowercase();
    if !lowercase.ends_with(".gguf") {
        return None;
    }
    let stem = &name[..name.len().saturating_sub(5)];
    let (with_index, total_text) = stem.rsplit_once("-of-")?;
    let (base, index_text) = with_index.rsplit_once('-')?;
    if base.is_empty()
        || index_text.len() != 5
        || total_text.len() != 5
        || !index_text
            .chars()
            .all(|character| character.is_ascii_digit())
        || !total_text
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return None;
    }
    let index = index_text.parse::<u32>().ok()?;
    let total = total_text.parse::<u32>().ok()?;
    if total <= 1 || index == 0 || index > total {
        return None;
    }
    Some(ModelShardDescriptor {
        base: base.to_string(),
        index,
        total,
    })
}

pub(crate) fn resolve_model_artifacts(path: &Path) -> Result<Vec<PathBuf>, ModelArtifactError> {
    if !path.is_file() {
        return Err(ModelArtifactError::Unavailable);
    }
    let Some(filename) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(ModelArtifactError::Unavailable);
    };
    let Some(target) = parse_model_shard_name(filename) else {
        return Ok(vec![path.to_path_buf()]);
    };
    let parent = path.parent().ok_or(ModelArtifactError::Unavailable)?;
    let entries = fs::read_dir(parent).map_err(|_| ModelArtifactError::Unavailable)?;
    let mut artifacts = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|_| ModelArtifactError::Unavailable)?;
        let file_type = entry
            .file_type()
            .map_err(|_| ModelArtifactError::Unavailable)?;
        if !file_type.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let Some(candidate) = parse_model_shard_name(name) else {
            continue;
        };
        if candidate.base != target.base {
            continue;
        }
        if candidate.total != target.total {
            return Err(ModelArtifactError::Incomplete);
        }
        if artifacts.insert(candidate.index, entry.path()).is_some() {
            return Err(ModelArtifactError::Incomplete);
        }
    }
    if artifacts.len() != target.total as usize
        || (1..=target.total).any(|index| !artifacts.contains_key(&index))
    {
        return Err(ModelArtifactError::Incomplete);
    }
    Ok(artifacts.into_values().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "llama-server-manager-artifacts-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, name.as_bytes()).unwrap();
            path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn parses_only_valid_multi_file_shard_names() {
        assert_eq!(
            parse_model_shard_name("Qwen-00001-of-00003.gguf"),
            Some(ModelShardDescriptor {
                base: "Qwen".into(),
                index: 1,
                total: 3,
            })
        );
        assert!(parse_model_shard_name("Qwen.gguf").is_none());
        assert!(parse_model_shard_name("Qwen-00000-of-00003.gguf").is_none());
        assert!(parse_model_shard_name("Qwen-00001-of-00001.gguf").is_none());
    }

    #[test]
    fn resolves_a_complete_ordered_set_from_any_shard() {
        let dir = TestDirectory::new();
        let first = dir.file("Qwen-00001-of-00003.gguf");
        let second = dir.file("Qwen-00002-of-00003.gguf");
        let third = dir.file("Qwen-00003-of-00003.gguf");
        assert_eq!(
            resolve_model_artifacts(&second).unwrap(),
            vec![first, second, third]
        );
    }

    #[test]
    fn rejects_incomplete_sets_without_affecting_single_files() {
        let dir = TestDirectory::new();
        let first = dir.file("Qwen-00001-of-00003.gguf");
        dir.file("Qwen-00003-of-00003.gguf");
        assert_eq!(
            resolve_model_artifacts(&first),
            Err(ModelArtifactError::Incomplete)
        );

        let single = dir.file("single.gguf");
        assert_eq!(resolve_model_artifacts(&single).unwrap(), vec![single]);
    }

    #[test]
    fn rejects_conflicting_totals_for_the_same_model_base() {
        let dir = TestDirectory::new();
        let first = dir.file("Qwen-00001-of-00002.gguf");
        dir.file("Qwen-00002-of-00002.gguf");
        dir.file("Qwen-00001-of-00003.gguf");

        assert_eq!(
            resolve_model_artifacts(&first),
            Err(ModelArtifactError::Incomplete)
        );
    }
}
