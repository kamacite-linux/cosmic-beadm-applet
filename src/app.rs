// SPDX-License-Identifier: MPL-2.0

use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{window::Id, Alignment, Length, Limits, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::theme;
use cosmic::widget::{divider, radio, text};
use futures_util::SinkExt;
use zbus::fdo::ObjectManagerProxy;
use zbus::zvariant::OwnedObjectPath;

use crate::dbus::BootEnvironmentProxy;
use crate::fl;

/// Represents a boot environment.
#[derive(Debug, Clone)]
pub struct BootEnvironment {
    /// The D-Bus object path for this boot environment.
    pub path: OwnedObjectPath,
    /// The name of this boot environment.
    pub name: String,
    /// A description for this boot environment, if any.
    pub description: Option<String>,
    /// Whether the system is currently booted into this boot environment.
    pub active: bool,
    /// Whether the system will reboot into this environment.
    pub next_boot: bool,
    /// Whether the system will reboot into this environment temporarily.
    pub boot_once: bool,
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
    /// List of boot environments.
    environments: Vec<BootEnvironment>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    SubscriptionChannel,
    BootSettingsClicked,
    ActivateEnvironment(zbus::zvariant::OwnedObjectPath),
    BootEnvironmentsLoaded(Vec<BootEnvironment>),
}

/// Query boot environments from D-Bus
async fn load_boot_environments() -> Result<Vec<BootEnvironment>, zbus::Error> {
    // Connect to the system bus
    let connection = zbus::Connection::system().await?;

    // Get the ObjectManager to list all boot environment objects
    let object_manager = ObjectManagerProxy::builder(&connection)
        .destination("ca.kamacite.BootEnvironments1")?
        .path("/ca/kamacite/BootEnvironments")?
        .build()
        .await?;

    // Get all managed objects
    let managed_objects = object_manager.get_managed_objects().await?;

    let mut environments = Vec::new();

    // Iterate through each object path
    for (path, _interfaces) in managed_objects {
        // Create a proxy for this boot environment
        let proxy = BootEnvironmentProxy::builder(&connection)
            .path(path.clone())?
            .build()
            .await?;

        // Query all properties
        let name = proxy.name().await?;
        let description_str = proxy.description().await?;
        let active = proxy.active().await?;
        let next_boot = proxy.next_boot().await?;
        let boot_once = proxy.boot_once().await?;
        let created = proxy.created().await?;

        // Convert to our BootEnvironment type
        let description = if description_str.is_empty() {
            None
        } else {
            Some(description_str)
        };

        environments.push(BootEnvironment {
            path,
            name,
            description,
            active,
            next_boot,
            boot_once,
            created,
        });
    }

    // Sort by creation time.
    environments.sort_by(|a, b| a.created.cmp(&b.created));

    Ok(environments)
}

/// Activate a boot environment by its D-Bus object path
async fn activate_boot_environment(path: OwnedObjectPath) -> Result<(), zbus::Error> {
    // Connect to the system bus
    let connection = zbus::Connection::system().await?;

    // Create a proxy for this boot environment
    let proxy = BootEnvironmentProxy::builder(&connection)
        .path(path)?
        .build()
        .await?;

    // Activate it (not temporary)
    proxy.activate(false).await?;
    Ok(())
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
    const APP_ID: &'static str = "ca.kamacite.cosmic-applet-boot-environment";

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
            // Start with empty list; will be populated from D-Bus
            environments: Vec::new(),
        };

        // Spawn task to load boot environments from D-Bus
        let task = Task::perform(load_boot_environments(), |result| {
            cosmic::Action::App(match result {
                Ok(environments) => Message::BootEnvironmentsLoaded(environments),
                Err(e) => {
                    eprintln!("Failed to load boot environments: {}", e);
                    // Return empty list on error
                    Message::BootEnvironmentsLoaded(Vec::new())
                }
            })
        });

        (app, task)
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
            .icon_button("drive-multidisk-symbolic")
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

            let env_row = radio(label, idx, selected_idx, |_idx| {
                Message::ActivateEnvironment(env.path.clone())
            });

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
            Message::BootSettingsClicked => {
                // Placeholder: would open boot settings configuration
                println!("Boot Settings clicked");
            }
            Message::BootEnvironmentsLoaded(environments) => {
                self.environments = environments;
            }
            Message::ActivateEnvironment(path) => {
                // Spawn task to activate via D-Bus and reload
                return Task::perform(
                    async move {
                        // Try to activate
                        if let Err(e) = activate_boot_environment(path).await {
                            eprintln!("Failed to activate boot environment: {}", e);
                        }
                        // Always reload to get current state
                        load_boot_environments().await
                    },
                    |result| {
                        cosmic::Action::App(match result {
                            Ok(environments) => Message::BootEnvironmentsLoaded(environments),
                            Err(e) => {
                                eprintln!("Failed to reload boot environments: {}", e);
                                Message::BootEnvironmentsLoaded(Vec::new())
                            }
                        })
                    },
                );
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
