//! Hub Terminal Panel - Tabbed terminal interface for The Hub

use gpui::{
    div, prelude::*, AnyElement, Context, Entity, EventEmitter, FocusHandle, Render, 
    Window, InteractiveElement, ParentElement, Styled, WeakEntity,
};
use std::collections::HashMap;
use theme::ActiveTheme;
use ui::{h_flex, v_flex, ButtonStyle, ButtonCommon, Clickable, IconButton, IconName, Tooltip};
use hub_terminal_engine::BlockTerminalView;

/// Hub Terminal Panel manages multiple terminal instances in tabs
pub struct HubTerminalPanel {
    terminal_tabs: HashMap<usize, Entity<BlockTerminalView>>,
    active_terminal_id: Option<usize>,
    next_terminal_id: usize,
    focus_handle: FocusHandle,
    workspace: WeakEntity<crate::Workspace>,
}

/// Events emitted by the Hub Terminal Panel
#[derive(Clone, Debug, PartialEq)]
pub enum HubTerminalEvent {
    TerminalAdded(usize),
    TerminalRemoved(usize),
    ActiveTerminalChanged(Option<usize>),
}

impl EventEmitter<HubTerminalEvent> for HubTerminalPanel {}

impl HubTerminalPanel {
    pub fn new(workspace: WeakEntity<crate::Workspace>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        
        let mut panel = Self {
            terminal_tabs: HashMap::new(),
            active_terminal_id: None,
            next_terminal_id: 1,
            focus_handle,
            workspace,
        };
        
        // Create the first terminal automatically
        panel.add_terminal(cx);
        
        panel
    }
    
    pub fn add_terminal(&mut self, cx: &mut Context<Self>) -> usize {
        let terminal_id = self.next_terminal_id;
        self.next_terminal_id += 1;
        
        // Create a Hub-enabled terminal block
        let terminal_block = cx.new_view(|cx| {
            BlockTerminalView::new()
        });
        
        self.terminal_tabs.insert(terminal_id, terminal_block);
        self.active_terminal_id = Some(terminal_id);
        
        cx.emit(HubTerminalEvent::TerminalAdded(terminal_id));
        cx.emit(HubTerminalEvent::ActiveTerminalChanged(self.active_terminal_id));
        cx.notify();
        
        terminal_id
    }
    
    pub fn remove_terminal(&mut self, terminal_id: usize, cx: &mut Context<Self>) {
        if self.terminal_tabs.remove(&terminal_id).is_some() {
            // If we removed the active terminal, switch to another one
            if self.active_terminal_id == Some(terminal_id) {
                self.active_terminal_id = self.terminal_tabs.keys().next().copied();
            }
            
            cx.emit(HubTerminalEvent::TerminalRemoved(terminal_id));
            cx.emit(HubTerminalEvent::ActiveTerminalChanged(self.active_terminal_id));
            cx.notify();
        }
    }
    
    pub fn set_active_terminal(&mut self, terminal_id: usize, cx: &mut Context<Self>) {
        if self.terminal_tabs.contains_key(&terminal_id) {
            self.active_terminal_id = Some(terminal_id);
            cx.emit(HubTerminalEvent::ActiveTerminalChanged(self.active_terminal_id));
            cx.notify();
        }
    }
    
    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .bg(cx.theme().colors().tab_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(
                h_flex()
                    .flex_1()
                    .children(
                        self.terminal_tabs
                            .keys()
                            .map(|&terminal_id| {
                                let is_active = self.active_terminal_id == Some(terminal_id);
                                
                                div()
                                    .flex()
                                    .items_center()
                                    .px_3()
                                    .py_1()
                                    .border_r_1()
                                    .border_color(cx.theme().colors().border)
                                    .when(is_active, |div| {
                                        div.bg(cx.theme().colors().tab_active_background)
                                    })
                                    .when(!is_active, |div| {
                                        div.bg(cx.theme().colors().tab_inactive_background)
                                            .hover(|div| div.bg(cx.theme().colors().element_hover))
                                    })
                                    .child(
                                        div()
                                            .flex_1()
                                            .cursor_pointer()
                                            .child(format!("Terminal {}", terminal_id))
                                            .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |this, _, _, cx| {
                                                this.set_active_terminal(terminal_id, cx);
                                            }))
                                    )
                                    .child(
                                        IconButton::new(("close_tab", terminal_id), IconName::Close)
                                            .style(ButtonStyle::Transparent)
                                            .tooltip(Tooltip::text("Close Terminal"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.remove_terminal(terminal_id, cx);
                                            }))
                                    )
                            })
                    )
            )
            .child(
                IconButton::new("add_terminal", IconName::Plus)
                    .style(ButtonStyle::Transparent)
                    .tooltip(Tooltip::text("New Terminal"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.add_terminal(cx);
                    }))
            )
    }
    
    fn render_active_terminal(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if let Some(terminal_id) = self.active_terminal_id {
            if let Some(terminal_block) = self.terminal_tabs.get(&terminal_id) {
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .overflow_hidden()
                        .child(terminal_block.clone())
                        .into_any_element()
                );
            }
        }
        
        None
    }
}

impl Render for HubTerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .child(
                // Terminal tabs (thinner)
                self.render_tabs(cx)
            )
            .child(
                // Active terminal content
                div()
                    .flex_1()
                    .overflow_hidden()
                    .when_some(self.render_active_terminal(cx), |div, terminal| {
                        div.child(terminal)
                    })
                    .when(self.active_terminal_id.is_none(), |this| {
                        this.flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .text_color(cx.theme().colors().text_muted)
                                    .child("No terminal open. Click + to add a new terminal.")
                            )
                    })
            )
    }
}