// SPDX-License-Identifier: MPL-2.0

use crate::config::Config;
use crate::fl;
use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_config::{self, CosmicConfigEntry};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{window::Id, Alignment, Length, Limits, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::theme;
use cosmic::widget::{divider, radio, text};
use futures_util::SinkExt;
use std::path::PathBuf;

/// Placeholder for boot environment root type.
/// TODO: Replace with actual Root type from your ZFS library.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Root {
    pub dataset: String,
}

/// Represents a boot environment.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct BootEnvironment {
    /// The name of this boot environment.
    pub name: String,
    /// The boot environment root.
    pub root: Root,
    /// The ZFS dataset GUID.
    pub guid: u64,
    /// A description for this boot environment, if any.
    pub description: Option<String>,
    /// If the boot environment is currently mounted, this is its mountpoint.
    pub mountpoint: Option<PathBuf>,
    /// Whether the system is currently booted into this boot environment.
    pub active: bool,
    /// Whether the system will reboot into this environment.
    pub next_boot: bool,
    /// Whether the system will reboot into this environment temporarily.
    pub boot_once: bool,
    /// Bytes on the filesystem associated with this boot environment.
    pub space: u64,
    /// Unix timestamp for when this boot environment was created.
    pub created: i64,
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The popup id.
    popup: Option<Id>,
    /// Configuration data that persists between application runs.
    config: Config,
    /// List of boot environments.
    environments: Vec<BootEnvironment>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    SubscriptionChannel,
    UpdateConfig(Config),
    BootSettingsClicked,
    ActivateEnvironment(usize),
}

/// Create a COSMIC application from the app model
impl cosmic::Application for AppModel {
    /// The async executor that will be used to run your application's commands.
    type Executor = cosmic::executor::Default;

    /// Data that your application receives to its init method.
    type Flags = ();

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "com.github.kamacite-linux.cosmic-applet-boot-environment";

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    /// Initializes the application with any given flags and startup commands.
    fn init(
        core: cosmic::Core,
        _flags: Self::Flags,
    ) -> (Self, Task<cosmic::Action<Self::Message>>) {
        // Construct the app model with the runtime's core.
        let app = AppModel {
            core,
            popup: None,
            config: cosmic_config::Config::new(Self::APP_ID, Config::VERSION)
                .map(|context| match Config::get_entry(&context) {
                    Ok(config) => config,
                    Err((_errors, config)) => {
                        // for why in errors {
                        //     tracing::error!(%why, "error loading app config");
                        // }

                        config
                    }
                })
                .unwrap_or_default(),
            environments: vec![
                BootEnvironment {
                    name: "default".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/default".to_string(),
                    },
                    guid: 1234567890,
                    description: Some("Current system configuration".to_string()),
                    mountpoint: Some(PathBuf::from("/")),
                    active: true,
                    next_boot: true,
                    boot_once: false,
                    space: 15_000_000_000,
                    created: 1704067200, // 2024-01-01
                },
                BootEnvironment {
                    name: "backup-2024-10-01".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/backup-2024-10-01".to_string(),
                    },
                    guid: 1234567891,
                    description: Some("Monthly backup from October".to_string()),
                    mountpoint: None,
                    active: false,
                    next_boot: false,
                    boot_once: false,
                    space: 14_500_000_000,
                    created: 1727740800, // 2024-10-01
                },
                BootEnvironment {
                    name: "testing-kernel-6.16".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/testing-kernel-6.16".to_string(),
                    },
                    guid: 1234567892,
                    description: Some("Testing new kernel version".to_string()),
                    mountpoint: None,
                    active: false,
                    next_boot: false,
                    boot_once: false,
                    space: 15_200_000_000,
                    created: 1728950400, // 2024-10-15
                },
                BootEnvironment {
                    name: "stable-snapshot".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/stable-snapshot".to_string(),
                    },
                    guid: 1234567893,
                    description: None,
                    mountpoint: None,
                    active: false,
                    next_boot: false,
                    boot_once: false,
                    space: 14_800_000_000,
                    created: 1720224000, // 2024-07-06
                },
                BootEnvironment {
                    name: "pre-upgrade".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/pre-upgrade".to_string(),
                    },
                    guid: 1234567894,
                    description: Some("Before system upgrade".to_string()),
                    mountpoint: None,
                    active: false,
                    next_boot: false,
                    boot_once: false,
                    space: 13_900_000_000,
                    created: 1715040000, // 2024-05-07
                },
                BootEnvironment {
                    name: "recovery".to_string(),
                    root: Root {
                        dataset: "rpool/ROOT/recovery".to_string(),
                    },
                    guid: 1234567895,
                    description: None,
                    mountpoint: None,
                    active: false,
                    next_boot: false,
                    boot_once: false,
                    space: 12_000_000_000,
                    created: 1699660800, // 2023-11-11
                },
            ],
        };

        (app, Task::none())
    }

    fn on_close_requested(&self, id: Id) -> Option<Message> {
        Some(Message::PopupClosed(id))
    }

    /// Describes the interface based on the current state of the application model.
    ///
    /// Application events will be processed through the view. Any messages emitted by
    /// events received by widgets will be passed to the update method.
    fn view(&self) -> Element<'_, Self::Message> {
        self.core
            .applet
            .icon_button("display-symbolic")
            .on_press(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let Spacing {
            space_xxs,
            space_s,
            space_m,
            ..
        } = theme::active().cosmic().spacing;

        // Build the column starting with boot environment rows
        let mut content = column![];

        // Add a row for each boot environment
        for (idx, env) in self.environments.iter().enumerate() {
            // Build status indicator text (localized)
            let status_text = if env.active && env.next_boot {
                Some(fl!("status-current"))
            } else if env.active {
                Some(fl!("status-current-temporary"))
            } else if env.next_boot {
                Some(fl!("status-next-boot"))
            } else if env.boot_once {
                Some(fl!("status-boot-once"))
            } else {
                None
            };

            // Build the label - either single line (name only) or two lines (description + name)
            let label: Element<'_, Message> = if let Some(desc) = &env.description {
                // Two-line layout: description (larger) on top, name (smaller) below
                column![text::body(desc), text::caption(&env.name),]
                    .spacing(2)
                    .into()
            } else {
                // Single line: just the name
                text::body(&env.name).into()
            };

            // Create status label on the right if present
            let status_label: Element<'_, Message> = if let Some(status) = status_text {
                text::caption(status).into()
            } else {
                cosmic::widget::Space::with_width(Length::Fixed(0.0)).into()
            };

            // Create the row with radio button
            // The radio button will be checked if this is the next_boot environment
            let selected_idx = self.environments.iter().position(|e| e.next_boot);

            let env_row = radio(label, idx, selected_idx, Message::ActivateEnvironment);

            // Wrap the radio button in a row with the status label
            let row_content = row![
                env_row,
                cosmic::widget::Space::with_width(Length::Fill),
                status_label,
            ]
            .align_y(Alignment::Center)
            .spacing(space_xxs)
            .padding([space_xxs, space_m]);

            content = content.push(row_content);
        }

        // Add divider before Boot Settings button
        content = content
            .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));

        // Add Boot Settings button at the bottom
        content = content.push(
            menu_button(text::body(fl!("boot-settings"))).on_press(Message::BootSettingsClicked),
        );

        let content = content.align_x(Alignment::Start).padding([8, 0]);

        self.core.applet.popup_container(content).into()
    }

    /// Register subscriptions for this application.
    ///
    /// Subscriptions are long-running async tasks running in the background which
    /// emit messages to the application through a channel. They are started at the
    /// beginning of the application, and persist through its lifetime.
    fn subscription(&self) -> Subscription<Self::Message> {
        struct MySubscription;

        Subscription::batch(vec![
            // Create a subscription which emits updates through a channel.
            Subscription::run_with_id(
                std::any::TypeId::of::<MySubscription>(),
                cosmic::iced::stream::channel(4, move |mut channel| async move {
                    _ = channel.send(Message::SubscriptionChannel).await;

                    futures_util::future::pending().await
                }),
            ),
            // Watch for application configuration changes.
            self.core()
                .watch_config::<Config>(Self::APP_ID)
                .map(|update| {
                    // for why in update.errors {
                    //     tracing::error!(?why, "app config error");
                    // }

                    Message::UpdateConfig(update.config)
                }),
        ])
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::SubscriptionChannel => {
                // For example purposes only.
            }
            Message::UpdateConfig(config) => {
                self.config = config;
            }
            Message::BootSettingsClicked => {
                // Placeholder: would open boot settings configuration
                println!("Boot Settings clicked");
            }
            Message::ActivateEnvironment(idx) => {
                // Update next_boot flags: deselect all, then select the chosen one
                if idx < self.environments.len() {
                    // First, clear next_boot from all environments
                    for env in &mut self.environments {
                        env.next_boot = false;
                        env.boot_once = false;
                    }

                    // Then set next_boot for the selected environment
                    self.environments[idx].next_boot = true;

                    // Log for debugging
                    let env = &self.environments[idx];
                    println!("Activated boot environment: {}", env.name);
                    if let Some(desc) = &env.description {
                        println!("  Description: {}", desc);
                    }
                }
            }
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let mut popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
                    popup_settings.positioner.size_limits = Limits::NONE
                        .max_width(372.0)
                        .min_width(300.0)
                        .min_height(200.0)
                        .max_height(1080.0);
                    get_popup(popup_settings)
                }
            }
            Message::PopupClosed(id) => {
                if self.popup.as_ref() == Some(&id) {
                    self.popup = None;
                }
            }
        }
        Task::none()
    }

    fn style(&self) -> Option<cosmic::iced_runtime::Appearance> {
        Some(cosmic::applet::style())
    }
}
