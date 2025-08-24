#[cfg(test)]
mod tests {
    use super::*;
    use agent_settings::AgentSettings;
    use assistant_tool::ToolWorkingSet;
    use gpui::{TestAppContext, Entity};
    use project::context_server_store::ContextServerStore;
    use project::Project;
    use settings::{Settings, SettingsStore};
    use std::sync::Arc;
    use workspace::Workspace;

    #[gpui::test]
    async fn test_agent_settings_modal_creation(cx: &mut TestAppContext) {
        cx.update(|cx| {
            settings::init(cx);
            AgentSettings::register(cx);
        });

        let fs = Arc::new(fs::FakeFs::new(cx.executor().clone()));
        let project = Project::test(fs.clone(), [], cx).await;
        let workspace = cx.new(|cx| Workspace::test_new(project.clone(), cx));
        let context_server_store = cx.new(|_| ContextServerStore::new());
        let tools = cx.new(|_| ToolWorkingSet::default());
        let language_registry = Arc::new(language::LanguageRegistry::test(cx.executor().clone()));

        cx.update(|cx| {
            let modal = AgentSettingsModal::new(
                fs,
                context_server_store,
                tools,
                language_registry,
                workspace.downgrade(),
                &mut cx.window_mut(),
                cx,
            );

            // Test that modal is created with default tab
            assert_eq!(modal.active_tab, SettingsTab::General);

            // Test that focus handle is properly initialized
            assert!(modal.focus_handle.is_focused(&cx.window()));
        });
    }

    #[gpui::test]
    async fn test_tab_switching(cx: &mut TestAppContext) {
        cx.update(|cx| {
            settings::init(cx);
            AgentSettings::register(cx);
        });

        let fs = Arc::new(fs::FakeFs::new(cx.executor().clone()));
        let project = Project::test(fs.clone(), [], cx).await;
        let workspace = cx.new(|cx| Workspace::test_new(project.clone(), cx));
        let context_server_store = cx.new(|_| ContextServerStore::new());
        let tools = cx.new(|_| ToolWorkingSet::default());
        let language_registry = Arc::new(language::LanguageRegistry::test(cx.executor().clone()));

        cx.update(|cx| {
            let mut modal = AgentSettingsModal::new(
                fs,
                context_server_store,
                tools,
                language_registry,
                workspace.downgrade(),
                &mut cx.window_mut(),
                cx,
            );

            // Test tab switching
            modal.set_active_tab(SettingsTab::Providers, cx);
            assert_eq!(modal.active_tab, SettingsTab::Providers);

            modal.set_active_tab(SettingsTab::ContextServers, cx);
            assert_eq!(modal.active_tab, SettingsTab::ContextServers);

            modal.set_active_tab(SettingsTab::Tools, cx);
            assert_eq!(modal.active_tab, SettingsTab::Tools);
        });
    }
}