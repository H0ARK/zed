use crate::pane::SelectedEntry;
use anyhow::Result;
use collections::HashMap;
use gpui::{App, AppContext, Context, Entity, EventEmitter, Global, WeakEntity};
use project::{Project, ProjectEntryId, WorktreeId};
use std::path::PathBuf;

pub fn init(project: Entity<Project>, cx: &mut App) {
    let navigation_state = cx.new(|cx| NavigationState::new(project, cx));
    cx.set_global(GlobalNavigationState(navigation_state));
}

struct GlobalNavigationState(Entity<NavigationState>);

impl Global for GlobalNavigationState {}

impl GlobalNavigationState {
    pub fn get(cx: &App) -> Entity<NavigationState> {
        cx.global::<Self>().0.clone()
    }

    pub fn get_or_init(project: Entity<Project>, cx: &mut App) -> Entity<NavigationState> {
        if cx.has_global::<Self>() {
            let existing_state = Self::get(cx);
            // Update the project reference but preserve existing state
            existing_state.update(cx, |state, _cx| {
                state.project = project.downgrade();
            });
            existing_state
        } else {
            init(project, cx);
            Self::get(cx)
        }
    }
}

#[derive(Clone, Debug)]
pub enum NavigationEvent {
    DirectoryChanged {
        old_directory: Option<PathBuf>,
        new_directory: Option<PathBuf>,
    },
    SelectionChanged {
        old_selection: Vec<SelectedEntry>,
        new_selection: Vec<SelectedEntry>,
    },
    DirectoryExpanded {
        worktree_id: WorktreeId,
        entry_id: ProjectEntryId,
    },
    DirectoryCollapsed {
        worktree_id: WorktreeId,
        entry_id: ProjectEntryId,
    },
}

pub struct NavigationState {
    project: WeakEntity<Project>,
    current_directory: Option<PathBuf>,
    selected_entries: Vec<SelectedEntry>,
    expanded_directories: HashMap<WorktreeId, Vec<ProjectEntryId>>,
}

impl EventEmitter<NavigationEvent> for NavigationState {}

impl NavigationState {
    pub fn get_or_init(project: Entity<Project>, cx: &mut App) -> Entity<NavigationState> {
        GlobalNavigationState::get_or_init(project, cx)
    }

    pub fn new(project: Entity<Project>, _cx: &mut Context<Self>) -> Self {
        let initial_directory = project
            .read(_cx)
            .first_project_directory(_cx)
            .or_else(|| project.read(_cx).active_project_directory(_cx).map(|p| p.to_path_buf()));

        Self {
            project: project.downgrade(),
            current_directory: initial_directory,
            selected_entries: Vec::new(),
            expanded_directories: HashMap::new(),
        }
    }

    pub fn current_directory(&self) -> Option<&PathBuf> {
        self.current_directory.as_ref()
    }

    pub fn set_current_directory(
        &mut self,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_current_directory(directory, true, cx)
    }

    pub fn update_current_directory_quietly(
        &mut self,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.update_current_directory(directory, false, cx)
    }

    fn update_current_directory(
        &mut self,
        directory: Option<PathBuf>,
        emit_event: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let old_directory = self.current_directory.clone();
        
        // Validate the directory exists in the project if provided
        if let Some(ref dir) = directory {
            if let Some(project) = self.project.upgrade() {
                let project = project.read(cx);
                if !project.find_worktree(dir, cx).is_some() {
                    anyhow::bail!("Directory not found in project: {}", dir.display());
                }
            }
        }

        self.current_directory = directory.clone();
        
        if emit_event {
            cx.emit(NavigationEvent::DirectoryChanged {
                old_directory,
                new_directory: directory,
            });
        }

        Ok(())
    }

    pub fn selected_entries(&self) -> &[SelectedEntry] {
        &self.selected_entries
    }

    pub fn set_selected_entries(
        &mut self,
        entries: Vec<SelectedEntry>,
        cx: &mut Context<Self>,
    ) {
        let old_selection = self.selected_entries.clone();
        self.selected_entries = entries.clone();
        
        cx.emit(NavigationEvent::SelectionChanged {
            old_selection,
            new_selection: entries,
        });
    }

    pub fn add_selected_entry(&mut self, entry: SelectedEntry, cx: &mut Context<Self>) {
        if !self.selected_entries.contains(&entry) {
            self.selected_entries.push(entry);
            cx.emit(NavigationEvent::SelectionChanged {
                old_selection: self.selected_entries[..self.selected_entries.len() - 1].to_vec(),
                new_selection: self.selected_entries.clone(),
            });
        }
    }

    pub fn remove_selected_entry(&mut self, entry: &SelectedEntry, cx: &mut Context<Self>) {
        let old_selection = self.selected_entries.clone();
        self.selected_entries.retain(|e| e != entry);
        
        if old_selection.len() != self.selected_entries.len() {
            cx.emit(NavigationEvent::SelectionChanged {
                old_selection,
                new_selection: self.selected_entries.clone(),
            });
        }
    }

    pub fn is_directory_expanded(&self, worktree_id: WorktreeId, entry_id: ProjectEntryId) -> bool {
        self.expanded_directories
            .get(&worktree_id)
            .map_or(false, |entries| entries.contains(&entry_id))
    }

    pub fn expand_directory(
        &mut self,
        worktree_id: WorktreeId,
        entry_id: ProjectEntryId,
        cx: &mut Context<Self>,
    ) {
        let entries = self.expanded_directories.entry(worktree_id).or_insert_with(Vec::new);
        if !entries.contains(&entry_id) {
            entries.push(entry_id);
            cx.emit(NavigationEvent::DirectoryExpanded {
                worktree_id,
                entry_id,
            });
        }
    }

    pub fn collapse_directory(
        &mut self,
        worktree_id: WorktreeId,
        entry_id: ProjectEntryId,
        cx: &mut Context<Self>,
    ) {
        if let Some(entries) = self.expanded_directories.get_mut(&worktree_id) {
            if let Some(index) = entries.iter().position(|&id| id == entry_id) {
                entries.remove(index);
                cx.emit(NavigationEvent::DirectoryCollapsed {
                    worktree_id,
                    entry_id,
                });
            }
        }
    }

    pub fn expanded_directories(&self) -> &HashMap<WorktreeId, Vec<ProjectEntryId>> {
        &self.expanded_directories
    }

    pub fn sync_from_project_panel(
        &mut self,
        directory: Option<PathBuf>,
        selection: Vec<SelectedEntry>,
        expanded_dirs: HashMap<WorktreeId, Vec<ProjectEntryId>>,
        cx: &mut Context<Self>,
    ) {
        // Update directory if different
        if self.current_directory != directory {
            let old_directory = self.current_directory.clone();
            self.current_directory = directory.clone();
            cx.emit(NavigationEvent::DirectoryChanged {
                old_directory,
                new_directory: directory,
            });
        }

        // Update selection if different
        if self.selected_entries != selection {
            let old_selection = self.selected_entries.clone();
            self.selected_entries = selection.clone();
            cx.emit(NavigationEvent::SelectionChanged {
                old_selection,
                new_selection: selection,
            });
        }

        // Update expanded directories
        self.expanded_directories = expanded_dirs;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use project::FakeFs;
    use std::sync::Arc;

    #[gpui::test]
    async fn test_navigation_state_directory_change(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/test",
            json!({
                "dir1": {
                    "file1.txt": "content1"
                },
                "dir2": {
                    "file2.txt": "content2"
                }
            }),
        )
        .await;

        let project = cx.update(|cx| {
            Project::test(Arc::new(fs), ["/test".as_ref()], cx)
        });

        let navigation_state = cx.new(|cx| NavigationState::new(project, cx));

        // Test setting directory
        navigation_state.update(cx, |state, cx| {
            state.set_current_directory(Some("/test/dir1".into()), cx).unwrap();
        });

        assert_eq!(
            navigation_state.read(cx).current_directory(),
            Some(&PathBuf::from("/test/dir1"))
        );
    }

    #[gpui::test]
    async fn test_navigation_state_selection(cx: &mut TestAppContext) {
        let fs = FakeFs::new(cx.executor());
        let project = cx.update(|cx| {
            Project::test(Arc::new(fs), ["/test".as_ref()], cx)
        });

        let navigation_state = cx.new(|cx| NavigationState::new(project, cx));

        let entry = SelectedEntry {
            worktree_id: WorktreeId::from_usize(1),
            entry_id: ProjectEntryId::from_proto(1),
        };

        navigation_state.update(cx, |state, cx| {
            state.add_selected_entry(entry, cx);
        });

        assert_eq!(navigation_state.read(cx).selected_entries().len(), 1);
        assert_eq!(navigation_state.read(cx).selected_entries()[0], entry);
    }
}