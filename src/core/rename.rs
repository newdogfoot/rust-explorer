use super::{
    model::{Fingerprint, is_package},
    operations::valid_name,
    worker::Cancellation,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug)]
pub enum RenamePattern {
    Replace {
        find: String,
        replacement: String,
    },
    Add {
        prefix: String,
        suffix: String,
    },
    Number {
        base: String,
        start: u64,
        digits: usize,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameSpec {
    pub name: Vec<u8>,
    pub before: Fingerprint,
}
#[derive(Clone, Debug)]
pub struct RenameRow {
    pub source: PathBuf,
    pub old_name: String,
    pub new_name: String,
    pub spec: Option<RenameSpec>,
    pub error: Option<String>,
    pub changed: bool,
}

// Match canonically equivalent Korean names, but copy untouched spans from
// the original bytes. In particular, replacing one word must not normalize
// every other NFD word in a filename.
fn replace_equivalent(source: &str, find: &str, replacement: &str) -> String {
    let find: String = find.nfc().collect();
    let mut normalized = String::new();
    let mut offsets = vec![Some(0)];
    for (start, cluster) in source.grapheme_indices(true) {
        let part: String = cluster.nfc().collect();
        let offset = normalized.len();
        normalized.push_str(&part);
        offsets.resize(normalized.len() + 1, None);
        offsets[offset] = Some(start);
        if part == cluster {
            for (byte, _) in part.char_indices() {
                offsets[offset + byte] = Some(start + byte);
            }
        }
        offsets[normalized.len()] = Some(start + cluster.len());
    }
    let mut result = String::new();
    let mut copied = 0;
    for (start, _) in normalized.match_indices(&find) {
        if let (Some(from), Some(to)) = (offsets[start], offsets[start + find.len()]) {
            result.push_str(&source[copied..from]);
            result.push_str(replacement);
            copied = to;
        }
    }
    result.push_str(&source[copied..]);
    result
}
fn name_for(
    path: &Path,
    directory: bool,
    pattern: &RenamePattern,
    index: usize,
) -> io::Result<String> {
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::other("UTF-8로 표현할 수 없는 이름은 개별 변경을 이용하세요"))?;
    let extension = if !directory || is_package(path) {
        super::archive::extension(path)
            .map(|suffix| &name[name.len() - suffix.len()..])
            .or_else(|| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|ext| &name[name.len() - ext.len() - 1..])
            })
    } else {
        None
    };
    let stem = extension.map_or(name, |ext| &name[..name.len() - ext.len()]);
    let ext = extension.unwrap_or_default();
    let result = match pattern {
        RenamePattern::Replace { find, replacement } => {
            if find.is_empty() {
                return Err(io::Error::other("찾을 내용을 입력하세요"));
            }
            replace_equivalent(name, find, replacement)
        }
        RenamePattern::Add { prefix, suffix } => format!("{prefix}{stem}{suffix}{ext}"),
        RenamePattern::Number {
            base,
            start,
            digits,
        } => {
            if base.trim().is_empty() {
                return Err(io::Error::other("새 이름을 입력하세요"));
            }
            let number = start
                .checked_add(index as u64)
                .ok_or_else(|| io::Error::other("번호 범위를 초과했습니다"))?;
            format!("{base} {number:0width$}{ext}", width = (*digits).min(20))
        }
    };
    valid_name(result.as_bytes())?;
    Ok(result)
}

pub fn preview(
    paths: &[PathBuf],
    pattern: &RenamePattern,
    cancel: &Cancellation,
) -> io::Result<Vec<RenameRow>> {
    let mut rows = Vec::with_capacity(paths.len());
    for (index, source) in paths.iter().enumerate() {
        cancel.check()?;
        let old_name = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut row = RenameRow {
            source: source.clone(),
            old_name: old_name.clone(),
            new_name: old_name,
            spec: None,
            error: None,
            changed: false,
        };
        let value = (|| {
            let metadata = fs::symlink_metadata(source)?;
            let name = name_for(source, metadata.is_dir(), pattern, index)?;
            Ok::<_, io::Error>((name, Fingerprint::from_metadata(&metadata)))
        })();
        match value {
            Ok((name, before)) => {
                row.changed = name != row.old_name;
                row.new_name = name.clone();
                row.spec = Some(RenameSpec {
                    name: name.into_bytes(),
                    before,
                });
            }
            Err(error) => row.error = Some(error.to_string()),
        }
        rows.push(row);
    }
    check_rows(&mut rows, cancel)?;
    Ok(rows)
}

pub fn check_rows(rows: &mut [RenameRow], cancel: &Cancellation) -> io::Result<()> {
    let mut directories = HashMap::new();
    let mut parents: HashMap<PathBuf, (PathBuf, bool, u64, bool)> = HashMap::new();
    let mut identities = Vec::with_capacity(rows.len());
    let mut sources = HashMap::new();
    let mut targets = HashMap::new();
    let mut duplicate_sources = HashSet::new();
    let mut duplicate_targets = HashSet::new();
    for (index, row) in rows.iter_mut().enumerate() {
        cancel.check()?;
        let result = (|| -> io::Result<Option<(PathBuf, bool)>> {
            let Some(spec) = &row.spec else {
                return Ok(None);
            };
            if !spec.before.matches(&row.source) {
                return Err(io::Error::other("미리보기 이후 원본이 변경되었습니다"));
            }
            let parent = row
                .source
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .ok_or_else(|| io::Error::other("볼륨 루트의 이름은 변경할 수 없습니다"))?;
            if !parents.contains_key(parent) {
                parents.insert(
                    parent.to_path_buf(),
                    (
                        fs::canonicalize(parent)?,
                        crate::platform::volume_case_sensitive(parent),
                        fs::metadata(parent)?.dev(),
                        crate::platform::location_info(parent).0,
                    ),
                );
            }
            let (canonical, sensitive, device, read_only) = &parents[parent];
            if spec.before.dev != *device {
                return Err(io::Error::other(
                    "저장장치 마운트 지점의 이름은 변경할 수 없습니다",
                ));
            }
            if *read_only {
                return Err(io::Error::other("읽기 전용 저장장치입니다"));
            }
            let name = valid_name(&spec.name)?;
            let target = parent.join(&name);
            match fs::symlink_metadata(&target) {
                Ok(_)
                    if target != row.source
                        && !crate::platform::case_alias(&row.source, &target) =>
                {
                    return Err(io::Error::other("같은 이름의 항목이 이미 있습니다"));
                }
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
            let fold = |s: &str| {
                if *sensitive {
                    s.nfc().collect::<String>()
                } else {
                    s.nfc().flat_map(char::to_lowercase).collect::<String>()
                }
            };
            let source_key = (canonical.clone(), fold(&row.old_name));
            if let Some(other) = sources.insert(source_key, index) {
                duplicate_sources.extend([other, index]);
            }
            let target_key = (canonical.clone(), fold(&row.new_name));
            if let Some(other) = targets.insert(target_key, index) {
                duplicate_targets.extend([other, index]);
            }
            let identity = canonical.join(row.source.file_name().unwrap());
            let directory = fs::symlink_metadata(&row.source)?.is_dir();
            if directory {
                directories.insert(fs::canonicalize(&row.source)?, index);
            }
            Ok(Some((identity, directory)))
        })();
        match result {
            Ok(identity) => identities.push(identity),
            Err(error) => {
                row.error = Some(error.to_string());
                identities.push(None);
            }
        }
    }
    for index in duplicate_sources {
        rows[index].error = Some("동일한 원본이 중복 선택되었습니다".into());
    }
    for index in duplicate_targets {
        rows[index].error = Some("변경할 이름이 서로 겹칩니다".into());
    }
    for (index, identity) in identities.iter().enumerate() {
        cancel.check()?;
        let Some((path, _)) = identity else {
            continue;
        };
        for parent in path.ancestors().skip(1) {
            if let Some(&ancestor) = directories.get(parent) {
                let error = "상위 폴더와 그 안의 항목은 나누어 변경하세요";
                rows[index].error = Some(error.into());
                rows[ancestor].error = Some(error.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn equivalent_matches_preserve_unmodified_spelling() {
        assert_eq!(
            replace_equivalent("한글_자료.txt", "한글", "문서"),
            "문서_자료.txt"
        );
        assert_eq!(
            replace_equivalent("한글_자료.txt", "한글", "문서"),
            "문서_자료.txt"
        );
        assert_eq!(
            replace_equivalent("👩‍💻 기록.txt", "기록", "일지"),
            "👩‍💻 일지.txt"
        );
    }
    #[test]
    fn numbering_preserves_file_extensions_and_folder_dots() {
        let p = RenamePattern::Number {
            base: "사진".into(),
            start: 7,
            digits: 3,
        };
        assert_eq!(
            name_for(Path::new("old.jpeg"), false, &p, 1).unwrap(),
            "사진 008.jpeg"
        );
        assert_eq!(
            name_for(Path::new("folder.v2"), true, &p, 1).unwrap(),
            "사진 008"
        );
        assert_eq!(
            name_for(Path::new("old.app"), true, &p, 1).unwrap(),
            "사진 008.app"
        );
        assert_eq!(
            name_for(Path::new("old.TAR.GZ"), false, &p, 1).unwrap(),
            "사진 008.TAR.GZ"
        );
        assert_eq!(
            name_for(
                Path::new("backup.tar.gz"),
                false,
                &RenamePattern::Add {
                    prefix: String::new(),
                    suffix: "_copy".into()
                },
                0
            )
            .unwrap(),
            "backup_copy.tar.gz"
        );
    }
}
