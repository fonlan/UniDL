use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::mpsc,
};

use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};

use crate::{
    logger,
    models::{DownloadStatus, DownloadTask, EngineKind},
    services,
};

struct WatchedTask {
    directory: PathBuf,
    path: PathBuf,
    missing: bool,
}

pub struct DownloadFileMonitor {
    watcher: RecommendedWatcher,
    events: mpsc::Receiver<notify::Result<Event>>,
    tasks: HashMap<String, WatchedTask>,
    directory_task_ids: HashMap<PathBuf, HashSet<String>>,
}

impl DownloadFileMonitor {
    pub fn new() -> Result<Self, notify::Error> {
        let (sender, events) = mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |event| {
                let _ = sender.send(event);
            },
            Config::default(),
        )?;
        Ok(Self {
            watcher,
            events,
            tasks: HashMap::new(),
            directory_task_ids: HashMap::new(),
        })
    }

    pub fn sync_tasks(&mut self, tasks: &[DownloadTask]) {
        self.drain_events();

        let active_ids = tasks
            .iter()
            .filter_map(|task| monitor_target(task).map(|_| task.id.clone()))
            .collect::<HashSet<_>>();
        let stale_ids = self
            .tasks
            .keys()
            .filter(|id| !active_ids.contains(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in stale_ids {
            self.remove_task(&id);
        }

        for task in tasks {
            let Some((directory, path)) = monitor_target(task) else {
                continue;
            };

            if self
                .tasks
                .get(&task.id)
                .is_some_and(|watched| watched.path == path && watched.directory == directory)
            {
                continue;
            }

            self.remove_task(&task.id);
            let missing = !path.exists();
            if !self.directory_task_ids.contains_key(&directory) {
                if let Err(error) = self.watcher.watch(&directory, RecursiveMode::NonRecursive) {
                    logger::warn(format!(
                        "download file monitor watch skipped: directory={}, error={error}",
                        directory.display()
                    ));
                    continue;
                }
            }
            self.directory_task_ids
                .entry(directory.clone())
                .or_default()
                .insert(task.id.clone());
            self.tasks.insert(
                task.id.clone(),
                WatchedTask {
                    directory,
                    path,
                    missing,
                },
            );
        }
    }

    pub fn apply_state(&mut self, tasks: &mut [DownloadTask]) {
        self.drain_events();
        for task in tasks {
            task.downloaded_file_missing = self
                .tasks
                .get(&task.id)
                .map(|watched| watched.missing)
                .unwrap_or(false);
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                Ok(event) => self.handle_event(event),
                Err(error) => logger::warn(format!("download file monitor event error: {error}")),
            }
        }
    }

    fn handle_event(&mut self, event: Event) {
        let directories = event
            .paths
            .iter()
            .filter_map(|path| event_directory(path))
            .collect::<HashSet<_>>();

        for directory in directories {
            let Some(task_ids) = self.directory_task_ids.get(&directory).cloned() else {
                continue;
            };
            for task_id in task_ids {
                if let Some(task) = self.tasks.get_mut(&task_id) {
                    task.missing = !task.path.exists();
                }
            }
        }
    }

    fn remove_task(&mut self, id: &str) {
        let Some(task) = self.tasks.remove(id) else {
            return;
        };
        let Some(ids) = self.directory_task_ids.get_mut(&task.directory) else {
            return;
        };
        ids.remove(id);
        if ids.is_empty() {
            self.directory_task_ids.remove(&task.directory);
            if let Err(error) = self.watcher.unwatch(&task.directory) {
                logger::warn(format!(
                    "download file monitor unwatch failed: directory={}, error={error}",
                    task.directory.display()
                ));
            }
        }
    }
}

fn monitor_target(task: &DownloadTask) -> Option<(PathBuf, PathBuf)> {
    if !matches!(task.engine, EngineKind::Aria2 | EngineKind::YtDlp)
        || task.status != DownloadStatus::Completed
    {
        return None;
    }

    let path = services::downloaded_entry_path(task);
    let directory = path.parent()?.to_path_buf();
    Some((directory, path))
}

fn event_directory(path: &Path) -> Option<PathBuf> {
    if path.is_dir() {
        Some(path.to_path_buf())
    } else {
        path.parent().map(Path::to_path_buf)
    }
}
