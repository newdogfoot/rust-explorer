use crate::core::{
    listing::{self, ScanUpdate},
    model::*,
    operations::*,
    store::Store,
    worker::{Cancellation, WorkerPool},
};
use crossbeam_channel::{Receiver, Sender, bounded};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

pub enum Event {
    AccessChanged {
        window: u64,
        origin: Option<(u64, PathBuf)>,
        paths: Vec<PathBuf>,
        detail: String,
        add_favorites: bool,
    },
    PinFolders {
        window: u64,
        group: crate::core::shortcuts::PinGroup,
        result: Result<Vec<PathBuf>, String>,
    },
    NetworkUnmounted {
        window: u64,
        result: Result<(), String>,
    },
    Applications {
        window: u64,
        request: u64,
        result: Result<Vec<crate::platform::OpenApplication>, String>,
    },
    FolderReady {
        window: u64,
        request: u64,
        tab: u64,
        origin: PathBuf,
        result: Result<PathBuf, String>,
    },
    RenamePreview {
        window: u64,
        dialog: u64,
        generation: u64,
        result: Result<Vec<crate::core::rename::RenameRow>, String>,
    },
    Scan {
        window: u64,
        tab: u64,
        generation: u64,
        update: ScanUpdate,
    },
    Metadata {
        window: u64,
        tab: u64,
        generation: u64,
        entries: Vec<Entry>,
    },
    Thumbnail {
        window: u64,
        tab: u64,
        generation: u64,
        id: EntryId,
        png: Option<Vec<u8>>,
        cost: usize,
    },
    Location {
        window: u64,
        tab: u64,
        generation: u64,
        read_only: bool,
        free_bytes: Option<u64>,
    },
    Filter {
        window: u64,
        tab: u64,
        generation: u64,
        filter_generation: u64,
        indices: Vec<usize>,
    },
    Watch {
        window: u64,
        tab: u64,
    },
    Sidebar {
        window: u64,
        parent: u64,
        entries: Vec<Entry>,
    },
    Volumes(Vec<crate::platform::Location>),
    Open {
        window: u64,
        path: PathBuf,
        directory: bool,
        error: Option<String>,
    },
    JobStarted {
        id: u64,
        label: String,
    },
    JobProgress {
        id: u64,
        progress: Progress,
    },
    JobDone {
        id: u64,
        journal: Journal,
    },
    Conflict {
        owner: u64,
        conflict: Conflict,
        reply: Sender<ConflictDecision>,
    },
    Message {
        window: u64,
        title: String,
        detail: String,
    },
    Recovered(Vec<Journal>),
    UndoDone {
        id: u64,
        owner: u64,
        result: Result<Journal, String>,
    },
    PreferencesChanged,
}

pub struct ScanRequest {
    pub window: u64,
    pub tab: u64,
    pub generation: u64,
    pub path: PathBuf,
    pub hidden: bool,
    pub sort: SortField,
    pub descending: bool,
    pub query: Option<String>,
    pub cancel: Cancellation,
}
pub struct FilterRequest {
    pub window: u64,
    pub tab: u64,
    pub generation: u64,
    pub filter_generation: u64,
    pub entries: Arc<Vec<Entry>>,
    pub query: String,
    pub cancel: Cancellation,
}

pub struct Runtime {
    pub sizing: WorkerPool,
    pub store: Store,
    pub sender: Sender<Event>,
    pub receiver: Receiver<Event>,
    pub io: WorkerPool,
    pub search: WorkerPool,
    pub metadata: WorkerPool,
    pub transfers: WorkerPool,
    pub persistence: WorkerPool,
    pub preferences: Arc<Mutex<Preferences>>,
    preference_revision: Arc<AtomicU64>,
    preference_save_pending: Arc<AtomicBool>,
    pub mounted_locations: Mutex<Vec<crate::platform::Location>>,
    pub jobs: Mutex<HashMap<u64, Cancellation>>,
    pub history: Mutex<Vec<Journal>>,
    pub recent_folders: Mutex<Vec<StoredPath>>,
    pub recent_servers: Mutex<Vec<String>>,
    pub folder_access: Mutex<crate::platform::folder_access::Memory>,
    access_ready: Arc<OnceLock<()>>,
    watchers: Mutex<HashMap<(u64, u64), RecommendedWatcher>>,
    sequence: AtomicU64,
}
impl Runtime {
    pub fn new(store: Store) -> Arc<Self> {
        let (sender, receiver) = bounded(256);
        let remembered: Vec<crate::platform::folder_access::SavedFolder> =
            store.get("folder_access").unwrap_or_default();
        let runtime = Arc::new(Self {
            folder_access: Mutex::new(Default::default()),
            access_ready: Arc::new(OnceLock::new()),
            sizing: WorkerPool::new("size", 1, 8),
            preferences: Arc::new(Mutex::new(store.preferences())),
            preference_revision: Arc::new(AtomicU64::new(0)),
            preference_save_pending: Arc::new(AtomicBool::new(false)),
            mounted_locations: Mutex::new(vec![]),
            recent_servers: Mutex::new(store.get("recent_servers").unwrap_or_default()),
            recent_folders: Mutex::new(store.get("recent_folders").unwrap_or_default()),
            store,
            sender,
            receiver,
            io: WorkerPool::new("navigation", 2, 32),
            search: WorkerPool::new("search", 1, 16),
            metadata: WorkerPool::new("metadata", 2, 32),
            transfers: WorkerPool::new("transfers", 1, 32),
            persistence: WorkerPool::new("state", 1, 64),
            jobs: Mutex::new(HashMap::new()),
            history: Mutex::new(vec![]),
            watchers: Mutex::new(HashMap::new()),
            sequence: AtomicU64::new(1),
        });
        let restoring = runtime.clone();
        let submitted = runtime.persistence.submit(move || {
            objc2::rc::autoreleasepool(|_| {
                let memory = crate::platform::folder_access::Memory::load(remembered);
                let _ = restoring.store.set("folder_access", &memory.saved);
                *restoring.folder_access.lock().unwrap() = memory;
                let _ = restoring.access_ready.set(());
            });
        });
        if submitted.is_err() {
            let _ = runtime.access_ready.set(());
        }
        runtime
    }
    pub fn next_id(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::Relaxed)
    }
    pub fn remember_access(
        self: &Arc<Self>,
        window: u64,
        origin: Option<(u64, PathBuf)>,
        urls: Vec<objc2::rc::Retained<objc2_foundation::NSURL>>,
        add_favorites: bool,
    ) -> Result<(), &'static str> {
        let runtime = self.clone();
        self.persistence.submit(move || {
            objc2::rc::autoreleasepool(|_| {
                let mut paths = Vec::new();
                let mut failures = 0;
                for url in urls
                    .into_iter()
                    .take(crate::platform::folder_access::MAX_FOLDERS)
                {
                    if let Some(path) = crate::platform::checked_file_url_path(&url) {
                        paths.push(path);
                    }
                    match crate::platform::folder_access::remember(url) {
                        Ok((saved, scope)) => {
                            if runtime
                                .folder_access
                                .lock()
                                .unwrap()
                                .insert(saved, scope)
                                .is_err()
                            {
                                failures += 1;
                            }
                        }
                        Err(_) => failures += 1,
                    }
                }
                let saved = runtime.folder_access.lock().unwrap().saved.clone();
                let detail = match runtime.store.set("folder_access", &saved) {
                    Err(e) => format!(
                        "폴더는 선택했지만 다음 실행을 위한 정보를 저장하지 못했습니다: {e}"
                    ),
                    Ok(()) if failures > 0 => format!(
                        "폴더 {}개 선택 · {failures}개는 다음 실행에 다시 선택해야 할 수 있습니다",
                        paths.len()
                    ),
                    Ok(()) => format!(
                        "폴더 {}개 선택 · 다음 실행에도 위치를 기억합니다",
                        paths.len()
                    ),
                };
                let _ = runtime.sender.send(Event::AccessChanged {
                    window,
                    origin,
                    paths,
                    detail,
                    add_favorites,
                });
            })
        })
    }
    pub fn forget_access(self: &Arc<Self>, window: u64) -> Result<(), &'static str> {
        let runtime = self.clone();
        self.persistence.submit(move || {
            let result = runtime.store.set(
                "folder_access",
                &Vec::<crate::platform::folder_access::SavedFolder>::new(),
            );
            let detail = match result {
                Ok(()) => {
                    runtime.folder_access.lock().unwrap().saved.clear();
                    "선택한 폴더 기록을 지웠습니다. macOS에서 이미 허용한 접근은 취소되지 않습니다."
                        .into()
                }
                Err(e) => format!("폴더 기록을 지우지 못했습니다: {e}"),
            };
            let _ = runtime.sender.send(Event::AccessChanged {
                window,
                origin: None,
                paths: vec![],
                detail,
                add_favorites: false,
            });
        })
    }
    pub fn remember_server(&self, address: String) {
        let addresses = {
            let mut saved = self.recent_servers.lock().unwrap();
            saved.retain(|old| old != &address);
            saved.insert(0, address);
            saved.truncate(10);
            saved.clone()
        };
        let store = self.store.clone();
        let _ = self.persistence.submit(move || {
            let _ = store.set("recent_servers", &addresses);
        });
    }
    pub fn remember_folder(self: &Arc<Self>, path: PathBuf) {
        let folders = {
            let mut folders = self.recent_folders.lock().unwrap();
            let stored = StoredPath::new(&path);
            if folders.first() == Some(&stored) {
                return;
            }
            folders.retain(|p| p != &stored);
            folders.insert(0, stored);
            folders.truncate(20);
            folders.clone()
        };
        let store = self.store.clone();
        let _ = self.persistence.submit(move || {
            let _ = store.set("recent_folders", &folders);
        });
    }
    pub fn open_folder(
        self: &Arc<Self>,
        window: u64,
        request: u64,
        tab: u64,
        origin: PathBuf,
        path: PathBuf,
    ) -> Result<(), &'static str> {
        let sender = self.sender.clone();
        self.io.submit(move || {
            let result = (|| {
                let metadata = std::fs::metadata(&path).map_err(|e| e.to_string())?;
                if !metadata.is_dir() {
                    return Err("선택한 경로가 폴더가 아닙니다".into());
                }
                std::fs::canonicalize(&path).map_err(|e| e.to_string())
            })();
            let _ = sender.send(Event::FolderReady {
                window,
                request,
                tab,
                origin,
                result,
            });
        })
    }
    pub fn scan(self: &Arc<Self>, request: ScanRequest) {
        let ScanRequest {
            window,
            tab,
            generation,
            path,
            hidden,
            sort,
            descending,
            query,
            cancel,
        } = request;
        let runtime = self.clone();
        let tx = self.sender.clone();
        let pool = if query.is_some() {
            &self.search
        } else {
            &self.io
        };
        let failure = pool.submit(move || {
            runtime.access_ready.wait();
            if cancel.cancelled() {
                return;
            }
            runtime.watch(window, tab, path.clone());
            let (read_only, free_bytes) = crate::platform::location_info(&path);
            send_cancel(
                &tx,
                Event::Location {
                    window,
                    tab,
                    generation,
                    read_only,
                    free_bytes,
                },
                &cancel,
            );
            let emit = |update| {
                send_cancel(
                    &tx,
                    Event::Scan {
                        window,
                        tab,
                        generation,
                        update,
                    },
                    &cancel,
                )
            };
            if let Some(query) = query {
                listing::search_directory(&path, &query, hidden, &cancel, emit);
            } else {
                listing::scan_directory(&path, hidden, sort, descending, &cancel, emit);
            }
        });
        if let Err(e) = failure {
            let _ = self.sender.try_send(Event::Scan {
                window,
                tab,
                generation,
                update: ScanUpdate::Failed(e.into()),
            });
        }
    }
    pub fn metadata(
        self: &Arc<Self>,
        window: u64,
        tab: u64,
        generation: u64,
        mut entries: Vec<Entry>,
    ) {
        let tx = self.sender.clone();
        let _ = self.metadata.submit(move || {
            for e in &mut entries {
                e.load_metadata();
            }
            let _ = tx.send(Event::Metadata {
                window,
                tab,
                generation,
                entries,
            });
        });
    }
    pub fn filter(self: &Arc<Self>, request: FilterRequest) {
        let FilterRequest {
            window,
            tab,
            generation,
            filter_generation,
            entries,
            query,
            cancel,
        } = request;
        let tx = self.sender.clone();
        let _ = self.search.submit(move || {
            let key = search_key(&query);
            let mut indices = Vec::new();
            for (i, e) in entries.iter().enumerate() {
                if i % 256 == 0 && cancel.cancelled() {
                    return;
                }
                if e.key.contains(&key) {
                    indices.push(i);
                }
            }
            send_cancel(
                &tx,
                Event::Filter {
                    window,
                    tab,
                    generation,
                    filter_generation,
                    indices,
                },
                &cancel,
            );
        });
    }
    fn watch(&self, window: u64, tab: u64, path: PathBuf) {
        let tx = self.sender.clone();
        let root = path.clone();
        let watcher = notify::recommended_watcher(move |result: notify::Result<notify::Event>| {
            if let Ok(e) = result {
                if e.kind.is_access() {
                    return;
                }
                if e.paths
                    .iter()
                    .any(|p| p == &root || p.parent() == Some(&root))
                {
                    let _ = tx.try_send(Event::Watch { window, tab });
                }
            }
        });
        if let Ok(mut w) = watcher
            && w.watch(&path, RecursiveMode::NonRecursive).is_ok()
        {
            if let Some(parent) = path.parent() {
                let _ = w.watch(parent, RecursiveMode::NonRecursive);
            }
            self.watchers.lock().unwrap().insert((window, tab), w);
        }
    }
    pub fn unwatch(&self, window: u64, tab: Option<u64>) {
        self.watchers
            .lock()
            .unwrap()
            .retain(|(w, t), _| *w != window || tab.is_some_and(|id| id != *t));
    }
    pub fn sidebar(self: &Arc<Self>, window: u64, parent: u64, path: PathBuf) {
        let tx = self.sender.clone();
        let _ = self.io.submit(move || {
            listing::scan_directory(
                &path,
                false,
                SortField::Name,
                false,
                &Cancellation::default(),
                |update| {
                    if let ScanUpdate::Complete { mut entries, .. } = update {
                        entries.retain(|e| e.kind == Kind::Folder);
                        let _ = tx.send(Event::Sidebar {
                            window,
                            parent,
                            entries,
                        });
                    }
                },
            );
        });
    }
    pub fn volumes(&self) {
        let tx = self.sender.clone();
        let _ = self.io.submit(move || {
            let paths = crate::platform::locations(&super::home_dir());
            let _ = tx.send(Event::Volumes(paths));
        });
    }
    pub fn open(&self, window: u64, path: PathBuf) {
        let tx = self.sender.clone();
        let _ = self.io.submit(move || {
            let result = std::fs::metadata(&path);
            let directory = result.as_ref().is_ok_and(|m| m.is_dir()) && !is_package(&path);
            let error = result.err().map(|e| e.to_string());
            let _ = tx.send(Event::Open {
                window,
                path,
                directory,
                error,
            });
        });
    }
    pub fn execute(self: &Arc<Self>, owner: u64, request: Operation) -> Result<u64, &'static str> {
        let id = self.next_id();
        let cancel = Cancellation::default();
        self.jobs.lock().unwrap().insert(id, cancel.clone());
        let runtime = self.clone();
        let tx = self.sender.clone();
        let label = request.label().to_owned();
        let result = self.transfers.submit(move || {
            let _ = tx.send(Event::JobStarted { id, label });
            let report = execute(
                request,
                &runtime.store,
                &cancel,
                |conflict| {
                    let (reply, rx) = bounded(1);
                    if tx
                        .send(Event::Conflict {
                            owner,
                            conflict,
                            reply,
                        })
                        .is_err()
                    {
                        return ConflictDecision {
                            choice: ConflictChoice::Cancel,
                            apply_all: false,
                        };
                    }
                    loop {
                        if cancel.cancelled() {
                            return ConflictDecision {
                                choice: ConflictChoice::Cancel,
                                apply_all: false,
                            };
                        }
                        match rx.recv_timeout(Duration::from_millis(100)) {
                            Ok(a) => return a,
                            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                                return ConflictDecision {
                                    choice: ConflictChoice::Cancel,
                                    apply_all: false,
                                };
                            }
                            Err(_) => {}
                        }
                    }
                },
                |progress| {
                    let _ = tx.try_send(Event::JobProgress { id, progress });
                },
            );
            runtime.jobs.lock().unwrap().remove(&id);
            let report = report.summary();
            {
                let mut history = runtime.history.lock().unwrap();
                history.insert(0, report.clone());
                history.truncate(100);
            }
            let _ = tx.send(Event::JobDone {
                id,
                journal: report,
            });
        });
        if result.is_err() {
            self.jobs.lock().unwrap().remove(&id);
        }
        result.map(|_| id)
    }
    pub fn cancel(&self, id: u64) {
        if let Some(c) = self.jobs.lock().unwrap().get(&id) {
            c.cancel();
        }
    }
    pub fn undo(self: &Arc<Self>, owner: u64) {
        self.undo_selected(owner, None);
    }
    pub fn undo_selected(self: &Arc<Self>, owner: u64, id: Option<String>) {
        let eligible: std::collections::HashSet<_> = self
            .history
            .lock()
            .unwrap()
            .iter()
            .filter(|j| j.can_undo() && id.as_ref().is_none_or(|id| id == &j.id))
            .map(|j| j.id.clone())
            .collect();
        let job_id = self.next_id();
        let cancel = Cancellation::default();
        self.jobs.lock().unwrap().insert(job_id, cancel.clone());
        let runtime = self.clone();
        let tx = self.sender.clone();
        let submitted = self.transfers.submit(move || {
            let _ = tx.send(Event::JobStarted {
                id: job_id,
                label: "실행 취소".into(),
            });
            let candidate = runtime
                .history
                .lock()
                .unwrap()
                .iter()
                .find(|j| j.can_undo() && eligible.contains(&j.id))
                .map(|j| j.id.clone());
            let result =
                if let Some(mut j) = candidate.and_then(|id| runtime.store.job::<Journal>(&id)) {
                    let result = undo(&mut j, &runtime.store, &cancel);
                    if result.is_err() && j.status == "undoing" {
                        j.status = "review".into();
                        let _ = runtime.store.save_job(&j.id, &j.status, &j);
                    }
                    let summary = j.summary();
                    if let Some(old) = runtime
                        .history
                        .lock()
                        .unwrap()
                        .iter_mut()
                        .find(|x| x.id == j.id)
                    {
                        *old = summary.clone();
                    }
                    result.map(|_| summary).map_err(|e| e.to_string())
                } else {
                    Err("되돌릴 수 있는 작업이 없습니다".into())
                };
            runtime.jobs.lock().unwrap().remove(&job_id);
            let _ = tx.send(Event::UndoDone {
                id: job_id,
                owner,
                result,
            });
        });
        if let Err(detail) = submitted {
            self.jobs.lock().unwrap().remove(&job_id);
            let _ = self.sender.send(Event::Message {
                window: owner,
                title: "실행 취소를 시작할 수 없습니다".into(),
                detail: detail.into(),
            });
        }
    }
    pub fn save_preferences(&self, prefs: Preferences) {
        {
            let mut latest = self.preferences.lock().unwrap();
            *latest = prefs;
            self.preference_revision.fetch_add(1, Ordering::SeqCst);
        }
        // Rapid preset selection writes the latest snapshot, without filling
        // the bounded worker queue or doing SQLite work on the UI thread.
        if self.preference_save_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let latest = self.preferences.clone();
        let revision = self.preference_revision.clone();
        let pending = self.preference_save_pending.clone();
        let store = self.store.clone();
        let tx = self.sender.clone();
        if let Err(error) = self.persistence.submit(move || {
            loop {
                let (prefs, version) = {
                    let prefs = latest.lock().unwrap();
                    (prefs.clone(), revision.load(Ordering::SeqCst))
                };
                if let Err(error) = store.set("preferences", &prefs) {
                    pending.store(false, Ordering::SeqCst);
                    let _ = tx.send(Event::Message {
                        window: 0,
                        title: "설정 저장 실패".into(),
                        detail: error,
                    });
                    break;
                }
                let _ = tx.send(Event::PreferencesChanged);
                pending.store(false, Ordering::SeqCst);
                if revision.load(Ordering::SeqCst) == version
                    || pending.swap(true, Ordering::SeqCst)
                {
                    break;
                }
            }
        }) {
            self.preference_save_pending.store(false, Ordering::SeqCst);
            let _ = self.sender.try_send(Event::Message {
                window: 0,
                title: "설정 저장 실패".into(),
                detail: error.into(),
            });
        }
    }
    pub fn save_tabs(&self, tabs: Vec<SavedTab>) {
        let store = self.store.clone();
        let _ = self.persistence.submit(move || {
            let _ = store.set("tabs", &tabs);
        });
    }
    pub fn recover(self: &Arc<Self>) {
        let runtime = self.clone();
        let _ = self.persistence.submit(move || {
            let interrupted = recover(&runtime.store);
            *runtime.history.lock().unwrap() =
                runtime.store.map_jobs(false, |_, status, mut j: Journal| {
                    j.status = status;
                    j.summary()
                });
            if !interrupted.is_empty() {
                let _ = runtime.sender.send(Event::Recovered(
                    interrupted.iter().map(Journal::summary).collect(),
                ));
            }
        });
    }
}
fn send_cancel(tx: &Sender<Event>, mut event: Event, cancel: &Cancellation) {
    loop {
        if cancel.cancelled() {
            return;
        }
        match tx.send_timeout(event, Duration::from_millis(30)) {
            Ok(()) => return,
            Err(crossbeam_channel::SendTimeoutError::Timeout(e)) => event = e,
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recent_server_addresses_restore_in_last_used_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("state.sqlite")).unwrap();
        let runtime = Runtime::new(store.clone());
        for i in 0..14 {
            runtime.remember_server(format!("smb://test-{i}/share"));
        }
        runtime.remember_server("smb://test-6/share".into());
        assert!(runtime.persistence.flush(Duration::from_secs(5)));
        let restored = Runtime::new(store);
        let saved = restored.recent_servers.lock().unwrap();
        assert_eq!(saved.len(), 10);
        assert_eq!(saved[0], "smb://test-6/share");
        assert_eq!(saved[1], "smb://test-13/share");
        assert_eq!(
            saved.iter().filter(|s| *s == "smb://test-6/share").count(),
            1
        );
    }
    #[test]
    fn recent_folders_keep_twenty_unique_paths_and_restore_from_storage() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("state.sqlite")).unwrap();
        let runtime = Runtime::new(store.clone());
        let paths: Vec<_> = (0..25)
            .map(|i| root.path().join(format!("폴더 {i}")))
            .collect();
        for path in &paths {
            runtime.remember_folder(path.clone());
        }
        runtime.remember_folder(paths[7].clone());
        runtime.remember_folder(paths[7].clone());
        let (done, completed) = bounded(1);
        runtime
            .persistence
            .submit(move || {
                done.send(()).unwrap();
            })
            .unwrap();
        completed.recv_timeout(Duration::from_secs(5)).unwrap();
        let expected: Vec<_> = std::iter::once(7)
            .chain((5..25).rev().filter(|i| *i != 7))
            .map(|i| StoredPath::new(&paths[i]))
            .collect();
        assert_eq!(*runtime.recent_folders.lock().unwrap(), expected);
        assert_eq!(
            store.get::<Vec<StoredPath>>("recent_folders").unwrap(),
            expected
        );
        let restored = Runtime::new(store);
        assert_eq!(*restored.recent_folders.lock().unwrap(), expected);
    }
    #[test]
    fn queued_undo_only_targets_operations_complete_when_requested() {
        let root = tempfile::tempdir().unwrap();
        let store = Store::open(&root.path().join("state.sqlite")).unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir(&src).unwrap();
        std::fs::create_dir(&dst).unwrap();
        let runtime = Runtime::new(store.clone());
        let make_move = |name: &str| {
            let file = src.join(name);
            std::fs::write(&file, name).unwrap();
            execute(
                Operation::transfer(OperationKind::Move, vec![file], dst.clone()),
                &store,
                &Cancellation::default(),
                |_| unreachable!(),
                |_| {},
            )
        };
        let old = make_move("old.txt");
        runtime.history.lock().unwrap().push(old.summary());
        let (release, hold) = bounded(1);
        runtime
            .transfers
            .submit(move || {
                hold.recv().unwrap();
            })
            .unwrap();
        runtime.undo(0);
        let new = make_move("new.txt");
        runtime.history.lock().unwrap().insert(0, new.summary());
        release.send(()).unwrap();
        loop {
            if let Event::UndoDone { result, .. } = runtime
                .receiver
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
            {
                assert!(result.is_ok(), "{result:?}");
                break;
            }
        }
        assert!(!dst.join("old.txt").exists());
        assert!(src.join("old.txt").exists());
        assert!(dst.join("new.txt").exists());
        assert!(runtime.jobs.lock().unwrap().is_empty());
    }
}
