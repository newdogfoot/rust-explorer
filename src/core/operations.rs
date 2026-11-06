use super::{model::*, store::Store, worker::Cancellation};
use crate::platform;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs, io,
    os::unix::{ffi::OsStringExt, fs::MetadataExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use unicode_normalization::UnicodeNormalization;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StoredPath(pub Vec<u8>);
impl StoredPath {
    pub fn new(p: &Path) -> Self {
        Self(path_bytes(p))
    }
    pub fn path(&self) -> PathBuf {
        path_from_bytes(self.0.clone())
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileTemplate {
    Text,
    RichText,
    Markdown,
    Csv,
    Json,
    Empty,
}
impl FileTemplate {
    pub const PRESETS: [Self; 5] = [
        Self::Text,
        Self::RichText,
        Self::Markdown,
        Self::Csv,
        Self::Json,
    ];

    pub fn menu_title(self) -> &'static str {
        match self {
            Self::Text => "텍스트 문서 (.txt)",
            Self::RichText => "서식 있는 텍스트 (.rtf)",
            Self::Markdown => "Markdown 문서 (.md)",
            Self::Csv => "CSV 파일 (.csv)",
            Self::Json => "JSON 파일 (.json)",
            Self::Empty => "기타 빈 파일…",
        }
    }

    fn default_name(self) -> &'static str {
        match self {
            Self::Text => "새 텍스트 문서.txt",
            Self::RichText => "새 서식 문서.rtf",
            Self::Markdown => "새 Markdown 문서.md",
            Self::Csv => "새 표.csv",
            Self::Json => "새 JSON 파일.json",
            Self::Empty => "새 파일",
        }
    }

    fn contents(self) -> &'static [u8] {
        match self {
            Self::Json => b"{}\n",
            Self::RichText => br"{\rtf1\ansi\deff0{\fonttbl{\f0 Helvetica;}}\f0\fs24 }",
            _ => b"",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Duplicate,
    Move,
    Rename,
    BatchRename,
    CreateFolder,
    CreateTextFile,
    CreateFile(FileTemplate),
    CompressZip,
    Compress7z,
    Trash,
    Extract,
    Drop,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Operation {
    #[serde(default)]
    pub completed_sources: Vec<StoredPath>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renames: Vec<super::rename::RenameSpec>,
    pub kind: OperationKind,
    pub sources: Vec<StoredPath>,
    pub destination: Option<StoredPath>,
    pub name: Option<Vec<u8>>,
}
impl Operation {
    pub fn transfer(kind: OperationKind, sources: Vec<PathBuf>, destination: PathBuf) -> Self {
        Self {
            kind,
            completed_sources: vec![],
            renames: vec![],
            sources: sources.iter().map(|p| StoredPath::new(p)).collect(),
            destination: Some(StoredPath::new(&destination)),
            name: None,
        }
    }
    pub fn label(&self) -> &'static str {
        match self.kind {
            OperationKind::Copy => "복사",
            OperationKind::Duplicate => "복제",
            OperationKind::Move => "이동",
            OperationKind::Rename => "이름 변경",
            OperationKind::BatchRename => "일괄 이름 변경",
            OperationKind::CreateFolder => "새 폴더",
            OperationKind::CreateTextFile => "새 텍스트 문서",
            OperationKind::CreateFile(_) => "파일 만들기",
            OperationKind::CompressZip => "ZIP 압축",
            OperationKind::Compress7z => "7z 압축",
            OperationKind::Trash => "휴지통으로 이동",
            OperationKind::Extract => "압축 풀기",
            OperationKind::Drop => "드래그 이동·복사",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConflictChoice {
    Replace,
    KeepBoth,
    Skip,
    Merge,
    Cancel,
}
#[derive(Clone, Copy, Debug)]
pub struct ConflictDecision {
    pub choice: ConflictChoice,
    pub apply_all: bool,
}
#[derive(Clone, Debug)]
pub struct Conflict {
    pub source: PathBuf,
    pub target: PathBuf,
    pub folders: bool,
    pub source_size: u64,
    pub target_size: u64,
    pub source_modified: i64,
    pub target_modified: i64,
}
#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub files: u64,
    pub bytes: u64,
    pub current: u64,
    pub total: u64,
    pub name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeStamp {
    entries: Vec<(Vec<u8>, Fingerprint)>,
    #[serde(default)]
    root_only: bool,
}
impl TreeStamp {
    pub fn capture(root: &Path) -> io::Result<Self> {
        let mut entries = vec![];
        let mut stack = vec![root.to_path_buf()];
        while let Some(path) = stack.pop() {
            let f = Fingerprint::read(&path)?;
            let dir = fs::symlink_metadata(&path)?.is_dir();
            entries.push((
                path_bytes(path.strip_prefix(root).unwrap_or(Path::new(""))),
                f,
            ));
            if dir {
                for e in fs::read_dir(path)? {
                    stack.push(e?.path());
                }
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(Self {
            entries,
            root_only: false,
        })
    }
    fn capture_root(root: &Path) -> io::Result<Self> {
        Ok(Self {
            entries: vec![(vec![], Fingerprint::read(root)?)],
            root_only: true,
        })
    }
    fn matches_after_trash(&self, root: &Path) -> bool {
        // macOS may update access-control/provenance xattrs after trashing.
        // Restore is an exclusive move: tolerate ctime-only changes while
        // retaining object identity, content metadata and full tree checks.
        Self::capture(root).is_ok_and(|current| {
            current.entries.len() == self.entries.len()
                && current.entries.into_iter().zip(&self.entries).all(
                    |((path, mut actual), (expected_path, expected))| {
                        actual.ctime = expected.ctime;
                        actual.ctime_ns = expected.ctime_ns;
                        path == *expected_path && actual == *expected
                    },
                )
        })
    }
    pub fn matches(&self, root: &Path) -> bool {
        (if self.root_only {
            Self::capture_root(root)
        } else {
            Self::capture(root)
        })
        .is_ok_and(|s| s.entries == self.entries)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum UndoStep {
    Created {
        path: StoredPath,
        stamp: TreeStamp,
    },
    Renamed {
        from: StoredPath,
        to: StoredPath,
        stamp: TreeStamp,
    },
    CrossMove {
        from: StoredPath,
        to: StoredPath,
        stamp: TreeStamp,
    },
}
impl UndoStep {
    fn current(&self) -> (&StoredPath, &TreeStamp) {
        match self {
            Self::Created { path, stamp } => (path, stamp),
            Self::Renamed { from, stamp, .. } | Self::CrossMove { from, stamp, .. } => {
                (from, stamp)
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PendingInstall {
    pub staged: StoredPath,
    pub target: StoredPath,
    pub backup: Option<StoredPath>,
    pub original: Option<StoredPath>,
    pub staged_id: Fingerprint,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Journal {
    pub id: String,
    pub request: Operation,
    pub status: String,
    pub undo: Vec<UndoStep>,
    pub pending: Option<PendingInstall>,
    pub errors: Vec<String>,
    pub completed: Vec<StoredPath>,
    pub skipped: usize,
}
impl Journal {
    fn advance_rename_fingerprint(&mut self, before: &Fingerprint, after: &Fingerprint) {
        let mut comparable = after.clone();
        comparable.ctime = before.ctime;
        comparable.ctime_ns = before.ctime_ns;
        if &comparable != before {
            return;
        }
        // Rename updates the ctime of every hard link. Only advance exact
        // old snapshots; unrelated content/identity changes still block undo.
        for step in &mut self.undo {
            let stamp = match step {
                UndoStep::Created { stamp, .. }
                | UndoStep::Renamed { stamp, .. }
                | UndoStep::CrossMove { stamp, .. } => stamp,
            };
            for (_, fingerprint) in &mut stamp.entries {
                if fingerprint == before {
                    *fingerprint = after.clone();
                }
            }
        }
        for spec in &mut self.request.renames {
            if &spec.before == before {
                spec.before = after.clone();
            }
        }
    }
    fn save(&self, store: &Store) -> io::Result<()> {
        store
            .save_job(&self.id, &self.status, self)
            .map_err(io::Error::other)
    }
    pub fn can_retry(&self) -> bool {
        self.pending.is_none()
            && matches!(
                self.status.as_str(),
                "failed" | "partial" | "cancelled" | "interrupted"
            )
    }
    pub fn can_undo(&self) -> bool {
        self.pending.is_none()
            && !self.undo.is_empty()
            && matches!(
                self.status.as_str(),
                "complete" | "partial" | "cancelled" | "interrupted"
            )
    }
    pub fn summary(&self) -> Self {
        let undo = self
            .undo
            .iter()
            .map(|step| match step {
                UndoStep::Created { path, .. } => UndoStep::Created {
                    path: path.clone(),
                    stamp: TreeStamp {
                        entries: vec![],
                        root_only: false,
                    },
                },
                UndoStep::Renamed { from, to, .. } => UndoStep::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                    stamp: TreeStamp {
                        entries: vec![],
                        root_only: false,
                    },
                },
                UndoStep::CrossMove { from, to, .. } => UndoStep::CrossMove {
                    from: from.clone(),
                    to: to.clone(),
                    stamp: TreeStamp {
                        entries: vec![],
                        root_only: false,
                    },
                },
            })
            .collect();
        Self {
            id: self.id.clone(),
            request: self.request.clone(),
            status: self.status.clone(),
            undo,
            pending: self.pending.clone(),
            errors: self.errors.clone(),
            completed: self.completed.clone(),
            skipped: self.skipped,
        }
    }
    pub fn recovery_paths(&self) -> Vec<PathBuf> {
        self.undo
            .iter()
            .filter_map(|s| match s {
                UndoStep::Renamed { from, .. }
                    if from
                        .path()
                        .file_name()
                        .is_some_and(|n| n.to_string_lossy().starts_with(".RustExplorer-")) =>
                {
                    Some(from.path())
                }
                _ => None,
            })
            .collect()
    }
}

struct Context<'a> {
    completed_sources: HashSet<Vec<u8>>,
    store: &'a Store,
    cancel: &'a Cancellation,
    journal: Journal,
    conflict: &'a mut dyn FnMut(Conflict) -> ConflictDecision,
    progress: &'a mut dyn FnMut(Progress),
    policy: Option<(bool, ConflictChoice)>,
    count: u64,
    bytes: u64,
    last: Instant,
}
impl Context<'_> {
    fn save(&self) -> io::Result<()> {
        self.journal.save(self.store)
    }
    fn notify(&mut self, path: &Path, current: u64, total: u64, force: bool) {
        if force || self.last.elapsed() > Duration::from_millis(60) {
            (self.progress)(Progress {
                files: self.count,
                bytes: self.bytes,
                current,
                total,
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            });
            self.last = Instant::now();
        }
    }
    fn resolve(&mut self, source: &Path, target: &Path) -> io::Result<ConflictChoice> {
        let s = fs::symlink_metadata(source)?;
        let t = fs::symlink_metadata(target)?;
        let folders = s.is_dir() && t.is_dir();
        if let Some((category, choice)) = self.policy
            && category == folders
        {
            return Ok(choice);
        }
        self.journal.status = "waiting".into();
        self.save()?;
        let answer = (self.conflict)(Conflict {
            source: source.into(),
            target: target.into(),
            folders,
            source_size: s.len(),
            target_size: t.len(),
            source_modified: s.mtime(),
            target_modified: t.mtime(),
        });
        self.journal.status = "running".into();
        self.save()?;
        if answer.apply_all {
            self.policy = Some((folders, answer.choice));
        }
        Ok(answer.choice)
    }
    fn remember(&mut self, step: UndoStep) -> io::Result<()> {
        self.journal.undo.push(step);
        self.save()
    }
    fn copy_tree(&mut self, source: &Path, target: &Path) -> io::Result<()> {
        self.cancel.check()?;
        let m = fs::symlink_metadata(source)?;
        if m.is_dir() {
            fs::create_dir(target)?;
            for entry in fs::read_dir(source)? {
                let e = entry?;
                self.copy_tree(&e.path(), &target.join(e.file_name()))
                    .map_err(|error| {
                        io::Error::new(error.kind(), format!("{}: {error}", e.path().display()))
                    })?;
            }
            platform::copy_directory_metadata(source, target)?;
        } else {
            let cancel = self.cancel.clone();
            platform::copy_node(source, target, &cancel, &mut |n, total| {
                self.notify(source, n, total, false)
            })?;
            self.count += 1;
            self.bytes = self.bytes.saturating_add(m.len());
            self.notify(source, m.len(), m.len(), true);
        }
        Ok(())
    }
    fn install(
        &mut self,
        stage: &Path,
        target: &Path,
        replace: bool,
        original: Option<&Path>,
    ) -> io::Result<()> {
        self.cancel.check()?;
        let backup = if replace {
            Some(hidden_sibling(target, "backup")?)
        } else {
            None
        };
        self.journal.pending = Some(PendingInstall {
            staged: StoredPath::new(stage),
            target: StoredPath::new(target),
            backup: backup.as_deref().map(StoredPath::new),
            original: original.map(StoredPath::new),
            staged_id: Fingerprint::read(stage)?,
        });
        self.save()?;
        if let Some(b) = &backup {
            platform::rename_exclusive(target, b)?;
            platform::sync_parent(target)?;
        }
        if let Err(e) = platform::rename_exclusive(stage, target) {
            if let Some(b) = &backup {
                let _ = platform::rename_exclusive(b, target);
            }
            return Err(e);
        }
        platform::sync_parent(target)?;
        if let Some(b) = backup {
            self.journal.undo.push(UndoStep::Renamed {
                from: StoredPath::new(&b),
                to: StoredPath::new(target),
                stamp: TreeStamp::capture(&b)?,
            });
        }
        // Renaming a directory only moves its entry; undo also moves it and
        // preserves its contents. It must not enumerate the entire subtree.
        let stamp = if original.is_some_and(|source| source == stage) {
            TreeStamp::capture_root(target)?
        } else {
            TreeStamp::capture(target)?
        };
        self.journal.undo.push(if let Some(source) = original {
            UndoStep::Renamed {
                from: StoredPath::new(target),
                to: StoredPath::new(source),
                stamp,
            }
        } else {
            UndoStep::Created {
                path: StoredPath::new(target),
                stamp,
            }
        });
        self.journal.pending = None;
        // Batch rename immediately saves the completed source and adjusted
        // hard-link fingerprints together. Until then, the durable pending
        // record still describes this rename for crash recovery.
        if self.journal.request.kind == OperationKind::BatchRename {
            Ok(())
        } else {
            self.save()
        }
    }
    fn install_new(&mut self, stage: &Path, proposed: &Path) -> io::Result<PathBuf> {
        for _ in 0..1000 {
            self.cancel.check()?;
            let target = if fs::symlink_metadata(proposed).is_ok() {
                unique_path(proposed)?
            } else {
                proposed.to_path_buf()
            };
            match self.install(stage, &target, false, None) {
                Ok(()) => return Ok(target),
                Err(error)
                    if error.kind() == io::ErrorKind::AlreadyExists
                        && self
                            .journal
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.staged_id.matches(stage)) =>
                {
                    self.journal.pending = None;
                    self.save()?;
                }
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other(
            "같은 이름의 파일이 계속 만들어지고 있습니다. 다시 시도하세요",
        ))
    }
    fn transfer(&mut self, source: &Path, target: &Path, moving: bool) -> io::Result<()> {
        self.cancel.check()?;
        if moving
            && let Some(parent) = source.parent().filter(|p| !p.as_os_str().is_empty())
            && fs::symlink_metadata(source)?.dev() != fs::metadata(parent)?.dev()
        {
            return Err(io::Error::other(
                "저장장치 마운트 지점은 이동하거나 이름을 바꿀 수 없습니다",
            ));
        }
        if self.completed_sources.contains(&path_bytes(source)) {
            self.journal.skipped += 1;
            return Ok(());
        }
        let same_location = source == target || same_parent_entry(source, target);
        if same_location && moving {
            return Ok(());
        }
        let mut target = if same_location {
            unique_path(target)?
        } else {
            target.to_path_buf()
        };
        validate_target(source, &target)?;
        let mut replace = false;
        if fs::symlink_metadata(&target).is_ok() && !platform::case_alias(source, &target) {
            match self.resolve(source, &target)? {
                ConflictChoice::Cancel => {
                    self.cancel.cancel();
                    self.cancel.check()?;
                }
                ConflictChoice::Skip => {
                    self.journal.skipped += 1;
                    return Ok(());
                }
                ConflictChoice::KeepBoth => target = unique_path(&target)?,
                ConflictChoice::Merge => {
                    if !fs::symlink_metadata(source)?.is_dir()
                        || !fs::symlink_metadata(&target)?.is_dir()
                    {
                        return Err(io::Error::other("폴더끼리만 병합할 수 있습니다"));
                    }
                    for entry in fs::read_dir(source)? {
                        let e = entry?;
                        if let Err(error) =
                            self.transfer(&e.path(), &target.join(e.file_name()), moving)
                        {
                            if error.kind() == io::ErrorKind::Interrupted
                                || self.journal.pending.is_some()
                            {
                                return Err(io::Error::new(
                                    error.kind(),
                                    format!("{}: {error}", e.path().display()),
                                ));
                            }
                            self.journal
                                .errors
                                .push(format!("{}: {error}", e.path().display()));
                            self.save()?;
                        }
                    }
                    if moving && fs::read_dir(source)?.next().is_none() {
                        let backup = hidden_sibling(source, "empty-folder")?;
                        platform::rename_exclusive(source, &backup)?;
                        self.remember(UndoStep::Renamed {
                            from: StoredPath::new(&backup),
                            to: StoredPath::new(source),
                            stamp: TreeStamp::capture(&backup)?,
                        })?;
                    }
                    return Ok(());
                }
                ConflictChoice::Replace => {
                    if fs::symlink_metadata(source)?.is_dir()
                        || fs::symlink_metadata(&target)?.is_dir()
                    {
                        return Err(io::Error::other(
                            "폴더 충돌은 병합 또는 둘 다 유지를 선택하세요",
                        ));
                    }
                    replace = true;
                }
            }
        }
        let m = fs::symlink_metadata(source)?;
        let parent = target
            .parent()
            .ok_or_else(|| io::Error::other("대상 폴더가 없습니다"))?;
        if moving && m.dev() == fs::metadata(parent)?.dev() {
            self.install(source, &target, replace, Some(source))?;
            self.count += 1;
            self.notify(source, m.len(), m.len(), true);
        } else {
            let before = if moving {
                Some(TreeStamp::capture(source)?)
            } else {
                None
            };
            let stage = hidden_sibling(&target, "stage")?;
            let result = self
                .copy_tree(source, &stage)
                .and_then(|_| self.install(&stage, &target, replace, None));
            if result.is_err() {
                let _ = remove_owned_stage(&stage);
            }
            result?;
            if moving {
                if !before.unwrap().matches(source) {
                    return Err(io::Error::other(
                        "복사 중 원본이 변경되어 원본과 완료된 복사본을 모두 유지했습니다",
                    ));
                }
                // Record reverse cross-volume move BEFORE deleting the original. A crash never makes
                // the journal claim the destination is a disposable copy after source removal.
                let last = self
                    .journal
                    .undo
                    .last_mut()
                    .ok_or_else(|| io::Error::other("이동 기록 누락"))?;
                *last = UndoStep::CrossMove {
                    from: StoredPath::new(&target),
                    to: StoredPath::new(source),
                    stamp: TreeStamp::capture(&target)?,
                };
                self.save()?;
                remove_exact(source)
                    .map_err(|e| io::Error::new(e.kind(), format!("원본 제거: {e}")))?;
                platform::sync_parent(source)?;
            }
        }
        self.journal.completed.push(StoredPath::new(source));
        self.save()
    }
}

// File URLs from Finder can use a canonically equivalent spelling. Compare
// names only; use the directory's original bytes, and verify the same inode.
// This runs on the transfer worker, with one directory pass per parent.
fn restore_source_spelling(sources: &mut [StoredPath], cancel: &Cancellation) {
    let mut parents: HashMap<PathBuf, Vec<usize>> = HashMap::new();
    for (index, stored) in sources.iter().enumerate() {
        let path = stored.path();
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| !n.is_ascii())
            && let Some(parent) = path.parent()
        {
            parents.entry(parent.to_owned()).or_default().push(index);
        }
    }
    for (parent, indices) in parents {
        if cancel.cancelled() {
            return;
        }
        let wanted: HashSet<String> = indices
            .iter()
            .filter_map(|&i| {
                sources[i]
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.nfc().collect())
            })
            .collect();
        let Ok(entries) = fs::read_dir(&parent) else {
            continue;
        };
        let mut names: HashMap<String, Vec<OsString>> = HashMap::new();
        for entry in entries.flatten() {
            if cancel.cancelled() {
                return;
            }
            let name = entry.file_name();
            if let Some(text) = name.to_str() {
                if text.is_ascii() {
                    continue;
                }
                let key: String = text.nfc().collect();
                if wanted.contains(&key) {
                    names.entry(key).or_default().push(name);
                }
            }
        }
        for index in indices {
            let path = sources[index].path();
            let Some(name) = path.file_name() else {
                continue;
            };
            let Some(text) = name.to_str() else {
                continue;
            };
            let key: String = text.nfc().collect();
            let Some(candidates) = names.get(&key) else {
                continue;
            };
            if candidates.iter().any(|n| n == name) {
                continue;
            }
            let Ok(original) = fs::symlink_metadata(&path) else {
                continue;
            };
            if original.ino() == 0 {
                continue;
            }
            for candidate in candidates {
                let corrected = parent.join(candidate);
                if let Ok(actual) = fs::symlink_metadata(&corrected)
                    && actual.dev() == original.dev()
                    && actual.ino() == original.ino()
                {
                    sources[index] = StoredPath::new(&corrected);
                    break;
                }
            }
        }
    }
}

pub fn execute(
    mut request: Operation,
    store: &Store,
    cancel: &Cancellation,
    mut conflict: impl FnMut(Conflict) -> ConflictDecision,
    mut progress: impl FnMut(Progress),
) -> Journal {
    restore_source_spelling(&mut request.sources, cancel);
    let journal = Journal {
        id: uuid::Uuid::new_v4().to_string(),
        request: request.clone(),
        status: "running".into(),
        undo: vec![],
        pending: None,
        errors: vec![],
        completed: vec![],
        skipped: 0,
    };
    let mut ctx = Context {
        completed_sources: request
            .completed_sources
            .iter()
            .map(|p| p.0.clone())
            .collect(),
        store,
        cancel,
        journal,
        conflict: &mut conflict,
        progress: &mut progress,
        policy: None,
        count: 0,
        bytes: 0,
        last: Instant::now(),
    };
    if let Err(e) = ctx.save() {
        ctx.journal.errors.push(e.to_string());
        ctx.journal.status = "failed".into();
        return ctx.journal;
    }
    let sources = if request.kind == OperationKind::BatchRename {
        request.sources.iter().map(StoredPath::path).collect()
    } else {
        deduplicate_sources(request.sources.iter().map(StoredPath::path).collect())
    };
    let mut run = || -> io::Result<()> {
        match request.kind {
            OperationKind::CreateFolder => {
                ctx.cancel.check()?;
                let parent = request
                    .destination
                    .as_ref()
                    .ok_or_else(|| io::Error::other("폴더를 선택하세요"))?
                    .path();
                let name = valid_name(request.name.as_deref().unwrap_or("새 폴더".as_bytes()))?;
                let target = parent.join(name);
                if request.name.is_some() && fs::symlink_metadata(&target).is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "같은 이름이 이미 있습니다",
                    ));
                }
                let stage = hidden_sibling(&target, "stage")?;
                fs::create_dir(&stage)?;
                let result = if request.name.is_none() {
                    ctx.install_new(&stage, &target)
                } else {
                    ctx.install(&stage, &target, false, None).map(|_| target)
                };
                if result.is_err() {
                    let _ = remove_owned_stage(&stage);
                }
                ctx.journal.completed.push(StoredPath::new(&result?));
            }
            OperationKind::CreateTextFile | OperationKind::CreateFile(_) => {
                use std::io::Write;
                // Older saved requests predate the choice of document format.
                let template = match request.kind {
                    OperationKind::CreateFile(template) => template,
                    _ => FileTemplate::Text,
                };
                ctx.cancel.check()?;
                let parent = request
                    .destination
                    .as_ref()
                    .ok_or_else(|| io::Error::other("폴더를 선택하세요"))?
                    .path();
                let name = valid_name(
                    request
                        .name
                        .as_deref()
                        .unwrap_or(template.default_name().as_bytes()),
                )?;
                let proposed = parent.join(name);
                let stage = hidden_sibling(&proposed, "stage")?;
                let result = (|| {
                    use std::os::unix::fs::OpenOptionsExt;
                    let mut file = fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .custom_flags(libc::O_NOFOLLOW)
                        .open(&stage)?;
                    file.write_all(template.contents())?;
                    file.sync_all()?;
                    ctx.install_new(&stage, &proposed)
                })();
                if result.is_err() {
                    let _ = remove_owned_stage(&stage);
                }
                ctx.journal.completed.push(StoredPath::new(&result?));
            }
            OperationKind::CompressZip | OperationKind::Compress7z => {
                if sources.is_empty() {
                    return Err(io::Error::other("압축할 파일이나 폴더를 선택하세요"));
                }
                ctx.cancel.check()?;
                let parent = request
                    .destination
                    .as_ref()
                    .ok_or_else(|| io::Error::other("압축파일을 만들 폴더를 선택하세요"))?
                    .path();
                let format = if request.kind == OperationKind::CompressZip {
                    super::archive::ArchiveFormat::Zip
                } else {
                    super::archive::ArchiveFormat::SevenZip
                };
                let name = valid_name(
                    request
                        .name
                        .as_deref()
                        .ok_or_else(|| io::Error::other("압축파일 이름이 필요합니다"))?,
                )?;
                let proposed = parent.join(name);
                let stage = hidden_sibling(&proposed, "stage")?;
                let cancellation = ctx.cancel.clone();
                let result = super::archive::create(
                    &sources,
                    &stage,
                    format,
                    &cancellation,
                    &mut |files, bytes, path| {
                        ctx.count = files;
                        ctx.bytes = bytes;
                        ctx.notify(path, 0, 0, files == 0 && bytes == 0);
                    },
                )
                .and_then(|_| ctx.install_new(&stage, &proposed));
                if result.is_err() {
                    let _ = remove_owned_stage(&stage);
                }
                ctx.journal.completed.push(StoredPath::new(&result?));
            }
            OperationKind::BatchRename => {
                if sources.is_empty() || sources.len() != request.renames.len() {
                    return Err(io::Error::other("이름 변경 계획이 올바르지 않습니다"));
                }
                let mut rows: Vec<_> = sources
                    .iter()
                    .zip(&request.renames)
                    .map(|(source, spec)| super::rename::RenameRow {
                        source: source.clone(),
                        old_name: source
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        new_name: String::from_utf8_lossy(&spec.name).into_owned(),
                        spec: Some(spec.clone()),
                        error: None,
                        changed: true,
                    })
                    .collect();
                super::rename::check_rows(&mut rows, ctx.cancel)?;
                if rows.iter().any(|row| row.error.is_some()) {
                    for row in rows.iter().filter(|row| row.error.is_some()).take(200) {
                        ctx.journal.errors.push(format!(
                            "{}: {}",
                            row.source.display(),
                            row.error.as_deref().unwrap()
                        ));
                    }
                    return Ok(());
                }
                for (index, source) in sources.iter().enumerate() {
                    ctx.cancel.check()?;
                    let spec = ctx.journal.request.renames[index].clone();
                    let target = source.parent().unwrap().join(valid_name(&spec.name)?);
                    if source == &target {
                        ctx.journal.completed.push(StoredPath::new(source));
                        ctx.journal.skipped += 1;
                        continue;
                    }
                    if !spec.before.matches(source) {
                        ctx.journal.errors.push(format!(
                            "{}: 실행 전에 원본이 변경되었습니다",
                            source.display()
                        ));
                        continue;
                    }
                    if let Err(error) = ctx.install(source, &target, false, Some(source)) {
                        // An exclusive-rename collision leaves the source intact.
                        // Clear only that known no-commit case; ambiguous failures
                        // remain journaled for recovery and stop the batch.
                        if error.kind() == io::ErrorKind::AlreadyExists
                            && spec.before.matches(source)
                        {
                            ctx.journal.pending = None;
                        }
                        return Err(io::Error::new(
                            error.kind(),
                            format!("{}: {error}", source.display()),
                        ));
                    }
                    let after = Fingerprint::read(&target)?;
                    ctx.journal.advance_rename_fingerprint(&spec.before, &after);
                    ctx.journal.completed.push(StoredPath::new(source));
                    ctx.count += 1;
                    ctx.save()?;
                    ctx.notify(&target, (index + 1) as u64, sources.len() as u64, true);
                }
            }
            OperationKind::Duplicate => {
                for source in &sources {
                    ctx.cancel.check()?;
                    if let Err(error) = ctx.transfer(source, source, false) {
                        if error.kind() == io::ErrorKind::Interrupted
                            || ctx.journal.pending.is_some()
                        {
                            return Err(error);
                        }
                        ctx.journal
                            .errors
                            .push(format!("{}: {error}", source.display()));
                    }
                }
            }
            OperationKind::Rename => {
                if sources.len() != 1 {
                    return Err(io::Error::other("이름을 변경할 항목 하나를 선택하세요"));
                }
                let name = valid_name(
                    request
                        .name
                        .as_deref()
                        .ok_or_else(|| io::Error::other("새 이름이 필요합니다"))?,
                )?;
                let source = &sources[0];
                let target = source
                    .parent()
                    .ok_or_else(|| io::Error::other("루트 이름은 변경할 수 없습니다"))?
                    .join(name);
                ctx.transfer(source, &target, true)?;
            }
            OperationKind::Extract => {
                if sources.len() != 1 {
                    return Err(io::Error::other("압축파일 하나를 선택하세요"));
                }
                ctx.cancel.check()?;
                let source = &sources[0];
                let parent = request
                    .destination
                    .as_ref()
                    .map(StoredPath::path)
                    .or_else(|| source.parent().map(Path::to_path_buf))
                    .ok_or_else(|| io::Error::other("압축을 풀 위치가 없습니다"))?;
                let mut target = parent.join(super::archive::folder_name(source));
                if fs::symlink_metadata(&target).is_ok() {
                    target = unique_path(&target)?;
                }
                let stage = hidden_sibling(&target, "stage")?;
                fs::create_dir(&stage)?;
                let cancellation = ctx.cancel.clone();
                let result = super::archive::extract(
                    source,
                    &stage,
                    &cancellation,
                    &mut |files, bytes, path| {
                        ctx.count = files;
                        ctx.bytes = bytes;
                        ctx.notify(path, 0, 0, false);
                    },
                )
                .and_then(|_| ctx.install(&stage, &target, false, None));
                if result.is_err() {
                    let _ = remove_owned_stage(&stage);
                }
                result?;
                ctx.journal.completed.push(StoredPath::new(source));
            }
            OperationKind::Copy | OperationKind::Move | OperationKind::Drop => {
                let dest = request
                    .destination
                    .as_ref()
                    .ok_or_else(|| io::Error::other("대상 폴더를 선택하세요"))?
                    .path();
                if !fs::metadata(&dest)?.is_dir() {
                    return Err(io::Error::other("대상이 폴더가 아닙니다"));
                }
                for source in &sources {
                    ctx.cancel.check()?;
                    let name = if sources.len() == 1 {
                        request.name.as_deref().map(valid_name).transpose()?
                    } else {
                        None
                    };
                    let result = source
                        .file_name()
                        .ok_or_else(|| io::Error::other("볼륨 루트는 복사할 수 없습니다"))
                        .and_then(|source_name| {
                            ctx.transfer(
                                source,
                                &dest.join(name.as_deref().unwrap_or(source_name)),
                                request.kind == OperationKind::Move
                                    || (request.kind == OperationKind::Drop
                                        && fs::symlink_metadata(source)?.dev()
                                            == fs::metadata(&dest)?.dev()),
                            )
                        });
                    if let Err(e) = result {
                        if e.kind() == io::ErrorKind::Interrupted {
                            return Err(e);
                        }
                        ctx.journal
                            .errors
                            .push(format!("{}: {e}", source.display()));
                        ctx.save()?;
                    }
                }
            }
            OperationKind::Trash => {
                for source in &sources {
                    ctx.cancel.check()?;
                    match platform::trash(source) {
                        Ok(trashed) => {
                            ctx.remember(UndoStep::Renamed {
                                from: StoredPath::new(&trashed),
                                to: StoredPath::new(source),
                                stamp: TreeStamp::capture(&trashed)?,
                            })?;
                            ctx.journal.completed.push(StoredPath::new(source));
                            ctx.count += 1;
                            ctx.notify(source, 1, 1, true);
                        }
                        Err(e) => ctx
                            .journal
                            .errors
                            .push(format!("{}: {e}", source.display())),
                    }
                }
            }
        }
        Ok(())
    };
    if let Err(e) = run() {
        ctx.journal.errors.push(e.to_string());
    }
    ctx.journal.status = if cancel.cancelled() {
        "cancelled"
    } else if ctx.journal.errors.is_empty() {
        "complete"
    } else if ctx.journal.undo.is_empty() {
        "failed"
    } else {
        "partial"
    }
    .into();
    if let Err(e) = ctx.save() {
        ctx.journal.errors.push(format!("작업 기록 저장 실패: {e}"));
        ctx.journal.status = "partial".into();
    }
    ctx.journal
}

pub fn undo(journal: &mut Journal, store: &Store, cancel: &Cancellation) -> io::Result<()> {
    cancel.check()?;
    if !journal.can_undo() {
        return Err(io::Error::other("되돌릴 수 있는 작업이 없습니다"));
    }
    let mut freed = HashSet::new();
    for step in journal.undo.iter().rev() {
        let (path, stamp) = step.current();
        let path = path.path();
        let unchanged = if journal.request.kind == OperationKind::Trash {
            stamp.matches_after_trash(&path)
        } else {
            stamp.matches(&path)
        };
        if !unchanged {
            return Err(io::Error::other(format!(
                "작업 이후 변경된 항목이 있어 되돌리기를 중단했습니다: {}",
                path.display()
            )));
        }
        if let UndoStep::Renamed { to, .. } | UndoStep::CrossMove { to, .. } = step {
            let p = to.path();
            if fs::symlink_metadata(&p).is_ok()
                && !freed.contains(&p)
                && !platform::case_alias(&path, &p)
            {
                return Err(io::Error::other(format!(
                    "원래 위치에 다른 항목이 있습니다: {}",
                    p.display()
                )));
            }
        }
        freed.insert(path);
    }
    journal.status = "undoing".into();
    journal.save(store)?;
    while let Some(step) = journal.undo.last().cloned() {
        cancel.check()?;
        match step {
            UndoStep::Created { path, .. } => {
                platform::trash(&path.path())?;
            }
            UndoStep::Renamed { from, to, .. } => {
                platform::rename_exclusive(&from.path(), &to.path())?;
            }
            UndoStep::CrossMove { from, to, .. } => {
                let mut request = Operation::transfer(
                    OperationKind::Move,
                    vec![from.path()],
                    to.path()
                        .parent()
                        .ok_or_else(|| io::Error::other("원래 폴더가 없습니다"))?
                        .into(),
                );
                request.name = to.path().file_name().map(|s| path_bytes(Path::new(s)));
                let mut result = execute(
                    request,
                    store,
                    cancel,
                    |_| ConflictDecision {
                        choice: ConflictChoice::Cancel,
                        apply_all: false,
                    },
                    |_| {},
                );
                if result.status != "complete" {
                    return Err(io::Error::other(result.errors.join("\n")));
                }
                result.status = "internal".into();
                result.save(store)?;
            }
        }
        journal.undo.pop();
        journal.save(store)?;
    }
    journal.status = "undone".into();
    journal.save(store)
}

pub fn recover(store: &Store) -> Vec<Journal> {
    let mut recovered = vec![];
    for (_, status, mut journal) in store.map_jobs(true, |id, status, j: Journal| (id, status, j)) {
        if status == "interrupted" && journal.pending.is_none() {
            continue;
        }
        if status == "undoing" {
            journal.status = "review".into();
            journal.errors.push("되돌리기 도중 종료되었습니다. 기록에 있는 원래 위치와 대상 위치를 확인하세요. 파일을 자동으로 다시 처리하지 않았습니다.".into());
            let _ = journal.save(store);
            recovered.push(journal);
            continue;
        }
        if !matches!(
            status.as_str(),
            "running" | "waiting" | "queued" | "interrupted"
        ) {
            continue;
        }
        journal.status = "interrupted".into();
        if let Some(pending) = journal.pending.clone() {
            let (stage, target) = (pending.staged.path(), pending.target.path());
            let rename_only = pending.original.as_ref() == Some(&pending.staged);
            let target_is_staged = Fingerprint::read(&target)
                .is_ok_and(|f| f.dev == pending.staged_id.dev && f.ino == pending.staged_id.ino);
            if rename_only {
                // An alias lookup can succeed even before a case-only rename.
                // Require the original entry to have disappeared, not merely
                // the target lookup to resolve to the same inode.
                let source_gone = match fs::symlink_metadata(&stage) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => true,
                    Ok(_) => platform::case_alias(&target, &stage),
                    _ => false,
                };
                if !target_is_staged || !source_gone {
                    if pending.staged_id.matches(&stage) {
                        journal.pending = None;
                    } else {
                        journal.status = "review".into();
                        journal.errors.push("중단된 이동·이름 변경의 원본·대상 상태를 확인할 수 없습니다. 파일을 보존했으므로 작업 기록에서 위치를 확인하세요.".into());
                    }
                    let _ = journal.save(store);
                    recovered.push(journal);
                    continue;
                }
            }
            if target_is_staged {
                let changed = Fingerprint::read(&target).map_or(true, |f| {
                    f.size != pending.staged_id.size
                        || f.mtime != pending.staged_id.mtime
                        || f.mtime_ns != pending.staged_id.mtime_ns
                        || f.mode != pending.staged_id.mode
                });
                if changed
                    || (!rename_only && fs::symlink_metadata(&target).is_ok_and(|m| m.is_dir()))
                {
                    journal.status = "review".into();
                    journal.errors.push("중단된 작업의 대상이 변경되었거나 폴더 내부 상태를 검증할 수 없습니다. 원본·대상·복구 파일을 보존했으므로 작업 기록에서 위치를 확인하세요.".into());
                    let _ = journal.save(store);
                    recovered.push(journal);
                    continue;
                }
                if let Some(b) = pending.backup {
                    let b = b.path();
                    if !journal.undo.iter().any(|s| s.current().0.path() == b)
                        && let Ok(stamp) = TreeStamp::capture(&b)
                    {
                        journal.undo.push(UndoStep::Renamed {
                            from: StoredPath::new(&b),
                            to: StoredPath::new(&target),
                            stamp,
                        });
                    }
                }
                if let Ok(stamp) = if rename_only {
                    TreeStamp::capture_root(&target)
                } else {
                    TreeStamp::capture(&target)
                } {
                    journal
                        .undo
                        .push(if let Some(original) = pending.original.clone() {
                            UndoStep::Renamed {
                                from: StoredPath::new(&target),
                                to: original,
                                stamp,
                            }
                        } else {
                            UndoStep::Created {
                                path: StoredPath::new(&target),
                                stamp,
                            }
                        });
                }
                if rename_only
                    && let Some(original) = pending.original.clone()
                    && !journal.completed.contains(&original)
                {
                    journal.completed.push(original);
                }
                if journal.request.kind == OperationKind::BatchRename
                    && let Ok(after) = Fingerprint::read(&target)
                {
                    journal.advance_rename_fingerprint(&pending.staged_id, &after);
                }
            } else if let Some(b) = pending.backup {
                let b = b.path();
                if fs::symlink_metadata(&target).is_err() {
                    let _ = platform::rename_exclusive(&b, &target);
                }
            }
            if pending.original.is_none()
                && Fingerprint::read(&stage)
                    .is_ok_and(|f| f.dev == pending.staged_id.dev && f.ino == pending.staged_id.ino)
            {
                let _ = remove_owned_stage(&stage);
            }
            journal.pending = None;
        }
        let _ = journal.save(store);
        recovered.push(journal);
    }
    recovered
}

pub fn retry_request(journal: &Journal) -> Operation {
    let mut request = journal.request.clone();
    if request.kind == OperationKind::BatchRename {
        let remaining: Vec<_> = request
            .sources
            .into_iter()
            .zip(request.renames)
            .filter(|(path, _)| !journal.completed.contains(path))
            .collect();
        (request.sources, request.renames) = remaining.into_iter().unzip();
    } else {
        request.sources.retain(|p| !journal.completed.contains(p));
    }
    for source in &journal.completed {
        if !request.completed_sources.contains(source) {
            request.completed_sources.push(source.clone());
        }
    }
    request
}
pub(crate) fn valid_name(name: &[u8]) -> io::Result<OsString> {
    if name.is_empty() || name == b"." || name == b".." || name.contains(&b'/') || name.contains(&0)
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "이름에는 / 또는 빈 이름을 사용할 수 없습니다",
        ))
    } else {
        Ok(OsString::from_vec(name.to_vec()))
    }
}
fn hidden_sibling(target: &Path, kind: &str) -> io::Result<PathBuf> {
    Ok(target
        .parent()
        .ok_or_else(|| io::Error::other("상위 폴더가 없습니다"))?
        .join(format!(".RustExplorer-{kind}-{}", uuid::Uuid::new_v4())))
}
pub fn unique_path(path: &Path) -> io::Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::other("상위 폴더가 없습니다"))?;
    let folder =
        fs::symlink_metadata(path).is_ok_and(|m| m.is_dir()) && !super::model::is_package(path);
    let stem = if folder {
        path.file_name()
    } else {
        path.file_stem()
    }
    .unwrap_or_default();
    let ext = if folder { None } else { path.extension() };
    for i in 2..100_000 {
        let mut name = stem.to_os_string();
        name.push(format!(" ({i})"));
        if let Some(ext) = ext {
            name.push(".");
            name.push(ext);
        }
        let p = parent.join(name);
        if fs::symlink_metadata(&p).is_err() {
            return Ok(p);
        }
    }
    Err(io::Error::other("사용할 수 있는 새 이름을 찾지 못했습니다"))
}
fn validate_target(source: &Path, target: &Path) -> io::Result<()> {
    let m = fs::symlink_metadata(source)?;
    if source == target {
        return Ok(());
    }
    if fs::symlink_metadata(target).is_ok_and(|t| t.dev() == m.dev() && t.ino() == m.ino())
        && !platform::case_alias(source, target)
    {
        return Err(io::Error::other("원본과 대상이 같은 파일입니다"));
    }
    if m.is_dir() {
        let s = fs::canonicalize(source)?;
        let p = fs::canonicalize(
            target
                .parent()
                .ok_or_else(|| io::Error::other("상위 폴더가 없습니다"))?,
        )?;
        if p.starts_with(&s) {
            return Err(io::Error::other(
                "폴더를 자기 자신의 하위 폴더에 넣을 수 없습니다",
            ));
        }
    }
    Ok(())
}
// Parent aliases such as /var and /private/var refer to the same entry.
// Requiring an identical leaf name keeps different hard links and case-only
// renames distinct. Runs on the worker, never during drag hover hit-testing.
fn same_parent_entry(a: &Path, b: &Path) -> bool {
    if a.file_name().is_none() || a.file_name() != b.file_name() {
        return false;
    }
    let (Some(ap), Some(bp)) = (a.parent(), b.parent()) else {
        return false;
    };
    match (fs::metadata(ap), fs::metadata(bp)) {
        (Ok(a), Ok(b)) => a.ino() != 0 && a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}
fn deduplicate_sources(mut sources: Vec<PathBuf>) -> Vec<PathBuf> {
    sources.sort();
    sources.dedup();
    // Descendants are contiguous after a lexicographic path sort. Avoid the
    // old all-pairs scan, which did billions of comparisons for large drops.
    let mut input = sources.into_iter().peekable();
    let mut result = Vec::new();
    let mut covered: Option<PathBuf> = None;
    while let Some(path) = input.next() {
        if covered
            .as_ref()
            .is_some_and(|parent| path.starts_with(parent))
        {
            continue;
        }
        covered = None;
        if input.peek().is_some_and(|next| next.starts_with(&path))
            && fs::symlink_metadata(&path).is_ok_and(|m| m.is_dir())
        {
            covered = Some(path.clone());
        }
        result.push(path);
    }
    result
}

fn remove_exact(path: &Path) -> io::Result<()> {
    platform::remove_item(path)
}
fn remove_owned_stage(path: &Path) -> io::Result<()> {
    if !path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .starts_with(".RustExplorer-stage-")
    {
        return Err(io::Error::other("임시 파일의 소유권을 확인할 수 없습니다"));
    }
    if fs::symlink_metadata(path).is_ok() {
        remove_exact(path)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    fn setup() -> (tempfile::TempDir, Store, PathBuf, PathBuf) {
        let t = tempfile::tempdir().unwrap();
        let s = Store::open(&t.path().join("db.sqlite")).unwrap();
        let a = t.path().join("src");
        let b = t.path().join("dst");
        fs::create_dir(&a).unwrap();
        fs::create_dir(&b).unwrap();
        (t, s, a, b)
    }
    fn run(
        s: &Store,
        kind: OperationKind,
        src: PathBuf,
        dst: PathBuf,
        choice: ConflictChoice,
    ) -> Journal {
        execute(
            Operation::transfer(kind, vec![src], dst),
            s,
            &Cancellation::default(),
            |_| ConflictDecision {
                choice,
                apply_all: false,
            },
            |_| {},
        )
    }
    #[test]
    fn deduplication_collapses_real_subtrees_but_preserves_links_and_similar_names() {
        let (temp, _, _, _) = setup();
        let parent = temp.path().join("folder");
        fs::create_dir_all(parent.join("inside")).unwrap();
        let link = temp.path().join("link");
        symlink(&parent, &link).unwrap();
        let similar = temp.path().join("folder-copy");
        let actual = deduplicate_sources(vec![
            parent.join("inside/file"),
            parent.clone(),
            parent.join("inside"),
            link.join("child"),
            link.clone(),
            similar.clone(),
            parent.clone(),
        ]);
        assert_eq!(
            actual,
            vec![parent, similar, link.clone(), link.join("child")]
        );
    }
    #[test]
    fn large_flat_selection_does_not_need_pairwise_path_checks() {
        let input = (0..100_000)
            .rev()
            .map(|n| PathBuf::from(format!("/nonexistent-selection-fixture/file-{n:06}")))
            .collect();
        let started = Instant::now();
        let result = deduplicate_sources(input);
        assert_eq!(result.len(), 100_000);
        eprintln!(
            "100000 selection preparation: {:.2} ms",
            started.elapsed().as_secs_f64() * 1000.
        );
    }
    #[test]
    fn copy_preserves_unicode_and_links() {
        let (_t, s, a, b) = setup();
        let p = a.join("한글😀");
        fs::create_dir(&p).unwrap();
        fs::write(p.join("한.txt"), b"hello").unwrap();
        symlink("한.txt", p.join("link")).unwrap();
        let j = run(
            &s,
            OperationKind::Copy,
            p,
            b.clone(),
            ConflictChoice::Cancel,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert_eq!(fs::read(b.join("한글😀/한.txt")).unwrap(), b"hello");
        assert!(
            fs::symlink_metadata(b.join("한글😀/link"))
                .unwrap()
                .is_symlink()
        );
    }
    #[test]
    fn move_and_undo() {
        let (_t, s, a, b) = setup();
        let p = a.join("a.txt");
        fs::write(&p, b"hello").unwrap();
        let mut j = run(
            &s,
            OperationKind::Move,
            p.clone(),
            b.clone(),
            ConflictChoice::Cancel,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert!(!p.exists());
        undo(&mut j, &s, &Cancellation::default()).unwrap();
        assert_eq!(fs::read(p).unwrap(), b"hello");
        assert!(!b.join("a.txt").exists());
    }
    #[test]
    fn replacement_can_be_undone_without_data_loss() {
        let (_t, s, a, b) = setup();
        let p = a.join("a");
        fs::write(&p, b"new").unwrap();
        fs::write(b.join("a"), b"old").unwrap();
        let mut j = run(
            &s,
            OperationKind::Move,
            p.clone(),
            b.clone(),
            ConflictChoice::Replace,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert_eq!(fs::read(b.join("a")).unwrap(), b"new");
        undo(&mut j, &s, &Cancellation::default()).unwrap();
        assert_eq!(fs::read(&p).unwrap(), b"new");
        assert_eq!(fs::read(b.join("a")).unwrap(), b"old");
    }
    #[test]
    fn undo_of_a_copy_refuses_changed_descendants() {
        let (_t, s, a, b) = setup();
        let p = a.join("folder");
        fs::create_dir(&p).unwrap();
        fs::write(p.join("child"), b"before").unwrap();
        let mut j = run(
            &s,
            OperationKind::Copy,
            p,
            b.clone(),
            ConflictChoice::Cancel,
        );
        fs::write(b.join("folder/child"), b"after").unwrap();
        assert!(undo(&mut j, &s, &Cancellation::default()).is_err());
        assert_eq!(fs::read(b.join("folder/child")).unwrap(), b"after");
    }
    #[test]
    fn copying_into_descendant_is_rejected() {
        let (_t, s, a, _b) = setup();
        let p = a.join("nested");
        fs::create_dir(&p).unwrap();
        let j = run(
            &s,
            OperationKind::Copy,
            a.clone(),
            p,
            ConflictChoice::Cancel,
        );
        assert_eq!(j.status, "failed");
        assert!(j.errors[0].contains("하위"));
    }
    #[test]
    fn keep_both_preserves_extension() {
        let (_t, s, a, b) = setup();
        let p = a.join("a.txt");
        fs::write(&p, b"new").unwrap();
        fs::write(b.join("a.txt"), b"old").unwrap();
        let j = run(
            &s,
            OperationKind::Copy,
            p,
            b.clone(),
            ConflictChoice::KeepBoth,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert_eq!(fs::read(b.join("a (2).txt")).unwrap(), b"new");
        assert_eq!(fs::read(b.join("a.txt")).unwrap(), b"old");
    }
    #[test]
    fn cancelled_copy_keeps_source() {
        let (_t, s, a, b) = setup();
        let p = a.join("a");
        fs::write(&p, b"data").unwrap();
        let c = Cancellation::default();
        c.cancel();
        let j = execute(
            Operation::transfer(OperationKind::Move, vec![p.clone()], b.clone()),
            &s,
            &c,
            |_| unreachable!(),
            |_| {},
        );
        assert_eq!(j.status, "cancelled");
        assert!(p.exists());
        assert!(!b.join("a").exists());
    }
    #[test]
    fn merge_does_not_replace_directory() {
        let (_t, s, a, b) = setup();
        fs::create_dir(a.join("folder")).unwrap();
        fs::create_dir(b.join("folder")).unwrap();
        fs::write(a.join("folder/new"), b"new").unwrap();
        fs::write(b.join("folder/old"), b"old").unwrap();
        let j = run(
            &s,
            OperationKind::Copy,
            a.join("folder"),
            b.clone(),
            ConflictChoice::Merge,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert!(b.join("folder/new").exists());
        assert!(b.join("folder/old").exists());
    }
    #[test]
    fn copy_in_same_folder_duplicates() {
        let (_t, s, a, _b) = setup();
        let p = a.join("a.txt");
        fs::write(&p, b"hello").unwrap();
        let j = run(
            &s,
            OperationKind::Copy,
            p.clone(),
            a.clone(),
            ConflictChoice::Cancel,
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert_eq!(fs::read(a.join("a (2).txt")).unwrap(), b"hello");
        assert!(p.exists());
    }
    #[test]
    fn case_only_rename_roundtrip() {
        let (_t, s, a, _b) = setup();
        let p = a.join("test.txt");
        fs::write(&p, b"data").unwrap();
        let r = Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::Rename,
            sources: vec![StoredPath::new(&p)],
            destination: None,
            name: Some(b"TEST.txt".to_vec()),
        };
        let mut j = execute(
            r,
            &s,
            &Cancellation::default(),
            |_| panic!("case-only rename should not ask for replacement"),
            |_| {},
        );
        assert_eq!(j.status, "complete", "{:?}", j.errors);
        assert!(
            fs::read_dir(&a)
                .unwrap()
                .any(|e| e.unwrap().file_name() == "TEST.txt")
        );
        undo(&mut j, &s, &Cancellation::default()).unwrap();
        assert!(
            fs::read_dir(a)
                .unwrap()
                .any(|e| e.unwrap().file_name() == "test.txt")
        );
    }
    #[test]
    fn hard_link_is_not_case_alias() {
        let (_t, _s, a, _b) = setup();
        let p = a.join("one");
        let q = a.join("two");
        fs::write(&p, b"data").unwrap();
        fs::hard_link(&p, &q).unwrap();
        assert!(!platform::case_alias(&p, &q));
        assert!(validate_target(&p, &q).is_err());
    }
    #[test]
    fn create_folder_records_completion() {
        let (_t, s, a, _b) = setup();
        let r = Operation {
            completed_sources: vec![],
            renames: vec![],
            kind: OperationKind::CreateFolder,
            sources: vec![],
            destination: Some(StoredPath::new(&a)),
            name: Some("새 폴더".as_bytes().to_vec()),
        };
        let j = execute(r, &s, &Cancellation::default(), |_| unreachable!(), |_| {});
        assert_eq!(j.status, "complete");
        assert_eq!(j.completed.len(), 1);
    }
    #[test]
    fn recovery_recognizes_committed_install() {
        let (_t, s, a, b) = setup();
        let stage = a.join(".RustExplorer-stage-test");
        fs::write(&stage, b"new").unwrap();
        let target = b.join("file");
        let fp = Fingerprint::read(&stage).unwrap();
        let req = Operation::transfer(OperationKind::Copy, vec![a.join("source")], b.clone());
        let j = Journal {
            id: "interrupted-install".into(),
            request: req,
            status: "running".into(),
            undo: vec![],
            pending: Some(PendingInstall {
                staged: StoredPath::new(&stage),
                target: StoredPath::new(&target),
                backup: None,
                original: None,
                staged_id: fp,
            }),
            errors: vec![],
            completed: vec![],
            skipped: 0,
        };
        j.save(&s).unwrap();
        platform::rename_exclusive(&stage, &target).unwrap();
        let recovered = recover(&s);
        assert_eq!(recovered.len(), 1);
        assert!(recovered[0].can_undo());
        assert_eq!(fs::read(target).unwrap(), b"new");
    }
    #[test]
    fn recovery_restores_backup_before_commit() {
        let (_t, s, a, b) = setup();
        let stage = b.join(".RustExplorer-stage-test");
        fs::write(&stage, b"new").unwrap();
        let target = b.join("file");
        let backup = b.join(".RustExplorer-backup-test");
        fs::write(&backup, b"old").unwrap();
        let j = Journal {
            id: "interrupted-before-install".into(),
            request: Operation::transfer(OperationKind::Copy, vec![a.join("source")], b),
            status: "running".into(),
            undo: vec![],
            pending: Some(PendingInstall {
                staged: StoredPath::new(&stage),
                target: StoredPath::new(&target),
                backup: Some(StoredPath::new(&backup)),
                original: None,
                staged_id: Fingerprint::read(&stage).unwrap(),
            }),
            errors: vec![],
            completed: vec![],
            skipped: 0,
        };
        j.save(&s).unwrap();
        recover(&s);
        assert_eq!(fs::read(target).unwrap(), b"old");
        assert!(!stage.exists());
    }
    #[test]
    fn recovery_does_not_make_changed_output_undoable() {
        let (_t, store, a, b) = setup();
        let stage = b.join(".RustExplorer-stage-modified");
        fs::write(&stage, b"new").unwrap();
        let target = b.join("file");
        let fp = Fingerprint::read(&stage).unwrap();
        let journal = Journal {
            id: "modified-output".into(),
            request: Operation::transfer(OperationKind::Copy, vec![a.join("source")], b),
            status: "running".into(),
            undo: vec![],
            pending: Some(PendingInstall {
                staged: StoredPath::new(&stage),
                target: StoredPath::new(&target),
                backup: None,
                original: None,
                staged_id: fp,
            }),
            errors: vec![],
            completed: vec![],
            skipped: 0,
        };
        journal.save(&store).unwrap();
        platform::rename_exclusive(&stage, &target).unwrap();
        fs::write(&target, b"user modified output").unwrap();
        let result = recover(&store);
        assert_eq!(result[0].status, "review");
        assert!(!result[0].can_undo());
        assert_eq!(fs::read(target).unwrap(), b"user modified output");
    }
    #[test]
    fn interrupted_undo_requires_review_without_replay() {
        let (_t, store, a, b) = setup();
        let target = b.join("file");
        fs::write(&target, b"keep me").unwrap();
        let journal = Journal {
            id: "undo-interrupted".into(),
            request: Operation::transfer(OperationKind::Copy, vec![a.join("source")], b),
            status: "undoing".into(),
            undo: vec![UndoStep::Created {
                path: StoredPath::new(&target),
                stamp: TreeStamp::capture(&target).unwrap(),
            }],
            pending: None,
            errors: vec![],
            completed: vec![],
            skipped: 0,
        };
        journal.save(&store).unwrap();
        let result = recover(&store);
        assert_eq!(result[0].status, "review");
        assert!(!result[0].can_undo());
        assert_eq!(fs::read(target).unwrap(), b"keep me");
        assert!(recover(&store).is_empty());
    }
    #[test]
    fn finder_decomposed_urls_preserve_original_file_and_link_names() {
        let (_t, store, a, b) = setup();
        let file = "파일📄.txt";
        let link = "링크🔗";
        fs::write(a.join(file), b"original").unwrap();
        symlink(file, a.join(link)).unwrap();
        let paths = vec![
            a.join(file.nfd().collect::<String>()),
            a.join(link.nfd().collect::<String>()),
        ];
        let result = execute(
            Operation::transfer(OperationKind::Copy, paths, b.clone()),
            &store,
            &Cancellation::default(),
            |_| ConflictDecision {
                choice: ConflictChoice::Cancel,
                apply_all: false,
            },
            |_| {},
        );
        assert_eq!(result.status, "complete", "{:?}", result.errors);
        let names: HashSet<_> = fs::read_dir(&b)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert!(names.contains(&OsString::from(file)));
        assert!(names.contains(&OsString::from(link)));
        assert_eq!(fs::read_link(b.join(link)).unwrap(), PathBuf::from(file));
        assert_eq!(fs::read(b.join(file)).unwrap(), b"original");
    }
    #[test]
    fn retry_merge_skips_files_already_installed() {
        let (_t, store, a, b) = setup();
        let src = a.join("folder");
        let dst = b.join("folder");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        for name in ["a.txt", "b.txt"] {
            fs::write(src.join(name), b"source").unwrap();
            fs::write(dst.join(name), b"destination").unwrap();
        }
        let mut conflicts = 0;
        let first = execute(
            Operation::transfer(OperationKind::Copy, vec![src.clone()], b.clone()),
            &store,
            &Cancellation::default(),
            |conflict| {
                let choice = if conflict.folders {
                    ConflictChoice::Merge
                } else {
                    conflicts += 1;
                    if conflicts == 1 {
                        ConflictChoice::KeepBoth
                    } else {
                        ConflictChoice::Cancel
                    }
                };
                ConflictDecision {
                    choice,
                    apply_all: false,
                }
            },
            |_| {},
        );
        assert_eq!(first.status, "cancelled");
        assert_eq!(first.completed.len(), 1);
        assert!(first.can_retry());
        let retry = execute(
            retry_request(&first),
            &store,
            &Cancellation::default(),
            |conflict| ConflictDecision {
                choice: if conflict.folders {
                    ConflictChoice::Merge
                } else {
                    ConflictChoice::KeepBoth
                },
                apply_all: false,
            },
            |_| {},
        );
        assert_eq!(retry.status, "complete", "{:?}", retry.errors);
        assert_eq!(retry.completed.len(), 1);
        assert_eq!(fs::read_dir(dst).unwrap().count(), 4);
    }
    #[test]
    fn cancelling_queued_undo_preserves_completed_record() {
        let (_t, store, a, b) = setup();
        let file = a.join("source");
        fs::write(&file, b"data").unwrap();
        let mut journal = run(
            &store,
            OperationKind::Copy,
            file,
            b.clone(),
            ConflictChoice::Cancel,
        );
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            undo(&mut journal, &store, &cancel).unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        assert_eq!(journal.status, "complete");
        assert!(journal.can_undo());
        assert_eq!(fs::read(b.join("source")).unwrap(), b"data");
    }
    #[test]
    fn trash_restore_tolerates_ctime_only_but_rejects_content_changes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("file");
        fs::write(&path, b"original").unwrap();
        let stamp = TreeStamp::capture(&path).unwrap();
        use std::os::unix::ffi::OsStrExt;
        let cpath = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        let attr = std::ffi::CString::new("com.rust-explorer.test").unwrap();
        let bytes = b"metadata";
        assert_eq!(
            unsafe {
                libc::setxattr(
                    cpath.as_ptr(),
                    attr.as_ptr(),
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    0,
                    0,
                )
            },
            0
        );
        assert!(!stamp.matches(&path));
        assert!(stamp.matches_after_trash(&path));
        fs::write(&path, b"modified").unwrap();
        assert!(!stamp.matches_after_trash(&path));
    }
}
