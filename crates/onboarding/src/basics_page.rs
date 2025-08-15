use std::sync::Arc;

use client::TelemetrySettings;
use fs::Fs;
use gpui::{App, IntoElement};
use settings::{BaseKeymap, Settings, update_settings_file};
use theme::{
    Appearance, SystemAppearance, ThemeMode, ThemeName, ThemeRegistry, ThemeSelection,
    ThemeSettings,
};
use ui::{
    ParentElement as _, RadioWithLabel, StatefulInteractiveElement, SwitchWithLabel,
    prelude::*, rems_from_px,
};
use vim_mode_setting::VimModeSetting;

use crate::theme_preview::{ThemePreviewStyle, ThemePreviewTile};

fn render_theme_section(cx: &mut App) -> impl IntoElement {
    let theme_selection = ThemeSettings::get_global(cx).theme_selection.clone();
    let system_appearance = theme::SystemAppearance::global(cx);
    let theme_selection = theme_selection.unwrap_or_else(|| ThemeSelection::Dynamic {
        mode: match *system_appearance {
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        },
        light: ThemeName("One Light".into()),
        dark: ThemeName("One Dark".into()),
    });

    let theme_mode = theme_selection
        .mode()
        .unwrap_or_else(|| match *system_appearance {
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        });

    return v_flex()
        .gap_2()
        .child(
            v_flex()
                .gap_2()
                .child(Label::new("Theme"))
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            RadioWithLabel::new(
                                "theme-light",
                                Label::new("Light"),
                                theme_mode == ThemeMode::Light,
                                |_, _, cx| {
                                    write_mode_change(ThemeMode::Light, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "theme-dark",
                                Label::new("Dark"),
                                theme_mode == ThemeMode::Dark,
                                |_, _, cx| {
                                    write_mode_change(ThemeMode::Dark, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "theme-system",
                                Label::new("System"),
                                theme_mode == ThemeMode::System,
                                |_, _, cx| {
                                    write_mode_change(ThemeMode::System, cx);
                                },
                            )
                        )
                ),
        )
        .child(
            h_flex()
                .gap_4()
                .justify_between()
                .children(render_theme_previews(&theme_selection, cx)),
        );

    fn render_theme_previews(
                theme_selection: &ThemeSelection,
        cx: &mut App,
    ) -> [impl IntoElement; 3] {
        let system_appearance = SystemAppearance::global(cx);
        let theme_registry = ThemeRegistry::global(cx);

        let theme_seed = 0xBEEF as f32;
        let theme_mode = theme_selection
            .mode()
            .unwrap_or_else(|| match *system_appearance {
                Appearance::Light => ThemeMode::Light,
                Appearance::Dark => ThemeMode::Dark,
            });
        let appearance = match theme_mode {
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
            ThemeMode::System => *system_appearance,
        };
        let current_theme_name = theme_selection.theme(appearance);

        const LIGHT_THEMES: [&'static str; 3] = ["One Light", "Ayu Light", "Gruvbox Light"];
        const DARK_THEMES: [&'static str; 3] = ["One Dark", "Ayu Dark", "Gruvbox Dark"];
        const FAMILY_NAMES: [SharedString; 3] = [
            SharedString::new_static("One"),
            SharedString::new_static("Ayu"),
            SharedString::new_static("Gruvbox"),
        ];

        let theme_names = match appearance {
            Appearance::Light => LIGHT_THEMES,
            Appearance::Dark => DARK_THEMES,
        };

        let themes = theme_names.map(|theme| theme_registry.get(theme).unwrap());

        let theme_previews = [0, 1, 2].map(|index| {
            let theme = &themes[index];
            let is_selected = theme.name == current_theme_name;
            let name = theme.name.clone();
            let colors = cx.theme().colors();

            v_flex()
                .w_full()
                .items_center()
                .gap_1()
                .child(
                    h_flex()
                        .id(name.clone())
                        .relative()
                        .w_full()
                        .border_2()
                        .border_color(colors.border_transparent)
                        .rounded(ThemePreviewTile::ROOT_RADIUS)
                        .map(|this| {
                            if is_selected {
                                this.border_color(colors.border_selected)
                            } else {
                                this.opacity(0.8).hover(|s| s.border_color(colors.border))
                            }
                        })
                        .focus(|mut style| {
                            style.border_color = Some(colors.border_focused);
                            style
                        })
                        .on_click({
                            let theme_name = theme.name.clone();
                            move |_, _, cx| {
                                write_theme_change(theme_name.clone(), theme_mode, cx);
                            }
                        })
                        .map(|this| {
                            if theme_mode == ThemeMode::System {
                                let (light, dark) = (
                                    theme_registry.get(LIGHT_THEMES[index]).unwrap(),
                                    theme_registry.get(DARK_THEMES[index]).unwrap(),
                                );
                                this.child(
                                    ThemePreviewTile::new(light, theme_seed)
                                        .style(ThemePreviewStyle::SideBySide(dark)),
                                )
                            } else {
                                this.child(
                                    ThemePreviewTile::new(theme.clone(), theme_seed)
                                        .style(ThemePreviewStyle::Bordered),
                                )
                            }
                        }),
                )
                .child(
                    Label::new(FAMILY_NAMES[index].clone())
                        .color(Color::Muted)
                        .size(LabelSize::Small),
                )
        });

        theme_previews
    }

    fn write_mode_change(mode: ThemeMode, cx: &mut App) {
        let fs = <dyn Fs>::global(cx);
        update_settings_file::<ThemeSettings>(fs, cx, move |settings, _cx| {
            settings.set_mode(mode);
        });
    }

    fn write_theme_change(theme: impl Into<Arc<str>>, theme_mode: ThemeMode, cx: &mut App) {
        let fs = <dyn Fs>::global(cx);
        let theme = theme.into();
        update_settings_file::<ThemeSettings>(fs, cx, move |settings, cx| {
            if theme_mode == ThemeMode::System {
                settings.theme = Some(ThemeSelection::Dynamic {
                    mode: ThemeMode::System,
                    light: ThemeName(theme.clone()),
                    dark: ThemeName(theme.clone()),
                });
            } else {
                let appearance = *SystemAppearance::global(cx);
                settings.set_theme(theme.clone(), appearance);
            }
        });
    }
}

fn render_telemetry_section(cx: &App) -> impl IntoElement {
    let fs = <dyn Fs>::global(cx);

    v_flex()
        .pt_6()
        .gap_4()
        .border_t_1()
        .border_color(cx.theme().colors().border_variant.opacity(0.5))
        .child(Label::new("Telemetry").size(LabelSize::Large))
        .child(SwitchWithLabel::new(
            "onboarding-telemetry-metrics",
            Label::new("Help Improve Zed"),
            if TelemetrySettings::get_global(cx).metrics {
                ui::ToggleState::Selected
            } else {
                ui::ToggleState::Unselected
            },
            {
            let fs = fs.clone();
            move |selection, _, cx| {
                let enabled = match selection {
                    ToggleState::Selected => true,
                    ToggleState::Unselected => false,
                    ToggleState::Indeterminate => { return; },
                };

                update_settings_file::<TelemetrySettings>(
                    fs.clone(),
                    cx,
                    move |setting, _| setting.metrics = Some(enabled),
                );
            }},
        ))
        .child(SwitchWithLabel::new(
            "onboarding-telemetry-crash-reports",
            Label::new("Help Fix Zed"),
            if TelemetrySettings::get_global(cx).diagnostics {
                ui::ToggleState::Selected
            } else {
                ui::ToggleState::Unselected
            },
            {
                let fs = fs.clone();
                move |selection, _, cx| {
                    let enabled = match selection {
                        ToggleState::Selected => true,
                        ToggleState::Unselected => false,
                        ToggleState::Indeterminate => { return; },
                    };

                    update_settings_file::<TelemetrySettings>(
                        fs.clone(),
                        cx,
                        move |setting, _| setting.diagnostics = Some(enabled),
                    );
                }
            }
        ))
}

fn render_base_keymap_section(cx: &mut App) -> impl IntoElement {
    let base_keymap = match BaseKeymap::get_global(cx) {
        BaseKeymap::VSCode => Some(0),
        BaseKeymap::JetBrains => Some(1),
        BaseKeymap::SublimeText => Some(2),
        BaseKeymap::Atom => Some(3),
        BaseKeymap::Emacs => Some(4),
        BaseKeymap::Cursor => Some(5),
        BaseKeymap::TextMate | BaseKeymap::None => None,
    };

    return v_flex()
        .gap_2()
        .child(Label::new("Base Keymap"))
        .child(
            v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            RadioWithLabel::new(
                                "keymap-vscode",
                                Label::new("VS Code"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::VSCode,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::VSCode, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "keymap-jetbrains",
                                Label::new("JetBrains"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::JetBrains,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::JetBrains, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "keymap-sublime",
                                Label::new("Sublime Text"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::SublimeText,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::SublimeText, cx);
                                },
                            )
                        )
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            RadioWithLabel::new(
                                "keymap-atom",
                                Label::new("Atom"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::Atom,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::Atom, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "keymap-emacs",
                                Label::new("Emacs"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::Emacs,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::Emacs, cx);
                                },
                            )
                        )
                        .child(
                            RadioWithLabel::new(
                                "keymap-cursor",
                                Label::new("Cursor"),
                                *BaseKeymap::get_global(cx) == BaseKeymap::Cursor,
                                |_, _, cx| {
                                    write_keymap_base(BaseKeymap::Cursor, cx);
                                },
                            )
                        )
                )
        );

    fn write_keymap_base(keymap_base: BaseKeymap, cx: &App) {
        let fs = <dyn Fs>::global(cx);

        update_settings_file::<BaseKeymap>(fs, cx, move |setting, _| {
            *setting = Some(keymap_base);
        });
    }
}

fn render_vim_mode_switch(cx: &mut App) -> impl IntoElement {
    let toggle_state = if VimModeSetting::get_global(cx).0 {
        ui::ToggleState::Selected
    } else {
        ui::ToggleState::Unselected
    };
    SwitchWithLabel::new(
        "onboarding-vim-mode",
        Label::new("Vim Mode"),
        toggle_state,
        {
            let fs = <dyn Fs>::global(cx);
            move |&selection, _, cx| {
                update_settings_file::<VimModeSetting>(fs.clone(), cx, move |setting, _| {
                    *setting = match selection {
                        ToggleState::Selected => Some(true),
                        ToggleState::Unselected => Some(false),
                        ToggleState::Indeterminate => None,
                    }
                });
            }
        },
    )
}

pub(crate) fn render_basics_page(cx: &mut App) -> impl IntoElement {
    v_flex()
        .gap_6()
        .child(render_theme_section(cx))
        .child(render_base_keymap_section(cx))
        .child(render_vim_mode_switch(cx))
        .child(render_telemetry_section(cx))
}
