// SPDX-License-Identifier: MPL-2.0

// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use std::collections::HashMap;

use cosmic::applet::{menu_button, padded_control};
use cosmic::cosmic_theme::Spacing;
use cosmic::iced::widget::{column, row};
use cosmic::iced::{window::Id, Alignment, Length, Subscription};
use cosmic::iced_winit::commands::popup::{destroy_popup, get_popup};
use cosmic::prelude::*;
use cosmic::theme;
use cosmic::widget::{divider, dropdown, text};
use futures_util::{SinkExt, StreamExt};
use zbus::fdo::ObjectManagerProxy;
use zbus::zvariant;

use crate::dbus::BootEnvironmentProxy;
use crate::fl;

/// Represents a boot environment object exposed on the bus.
#[derive(Debug, Clone)]
pub struct BootEnvironmentObject {
    /// The D-Bus object path foight n boot environment.
    pub path: zvariant::OwnedObjectPath,
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

impl BootEnvironmentObject {
    /// Construct a BootEnvironmentObject from a D-Bus dictionary of properties.
    pub fn from_properties<'a, K, V>(
        path: zvariant::OwnedObjectPath,
        props: &'a HashMap<K, V>,
    ) -> Result<Self, zbus::Error>
    where
        K: std::borrow::Borrow<str> + Eq + std::hash::Hash,
        V: std::borrow::Borrow<zvariant::Value<'a>>,
    {
        // This is a gross but useful wrapper around downcast_ref().
        fn get_prop<'a, T, K, V>(
            props: &'a HashMap<K, V>,
            name: &str,
        ) -> Result<T, zbus::zvariant::Error>
        where
            K: std::borrow::Borrow<str> + Eq + std::hash::Hash,
            V: std::borrow::Borrow<zvariant::Value<'a>>,
            T: TryFrom<&'a zvariant::Value<'a>>,
            <T as TryFrom<&'a zvariant::Value<'a>>>::Error: Into<zvariant::Error>,
        {
            props
                .get(name)
                .ok_or(zbus::zvariant::Error::IncorrectType)?
                .borrow()
                .downcast_ref()
        }

        // Special handling for optional properties.
        let description_str: String = get_prop(&props, "Description")?;
        let description = if description_str.is_empty() {
            None
        } else {
            Some(description_str)
        };

        Ok(BootEnvironmentObject {
            path,
            name: get_prop(&props, "Name")?,
            description,
            active: get_prop(&props, "Active")?,
            next_boot: get_prop(&props, "NextBoot")?,
            boot_once: get_prop(&props, "BootOnce")?,
            created: get_prop(&props, "Created")?,
        })
    }
}

/// The application model stores app-specific state used to describe its interface and
/// drive its logic.
pub struct AppModel {
    /// Application state which is managed by the COSMIC runtime.
    core: cosmic::Core,
    /// The popup id.
    popup: Option<Id>,
    /// List of boot environments.
    environments: Vec<BootEnvironmentObject>,
    /// The active D-Bus connection, if any.
    conn: Option<zbus::Connection>,
}

/// Messages emitted by the application and its widgets.
#[derive(Debug, Clone)]
pub enum Message {
    TogglePopup,
    PopupClosed(Id),
    BootSettingsClicked,
    ActivateEnvironment(zvariant::OwnedObjectPath),
    BootEnvironmentsLoaded(Vec<BootEnvironmentObject>),
    Connected(zbus::Connection),
    Added(BootEnvironmentObject),
    Removed(zvariant::OwnedObjectPath),
    BootEnvironmentsModified,
}

/// Query boot environments from D-Bus using the provided connection
async fn load_boot_environments(
    connection: &zbus::Connection,
) -> Result<Vec<BootEnvironmentObject>, zbus::Error> {
    // Get the ObjectManager to list all boot environment objects
    let object_manager = ObjectManagerProxy::builder(connection)
        .destination("ca.kamacite.BootEnvironments1")?
        .path("/ca/kamacite/BootEnvironments")?
        .build()
        .await?;

    let mut environments = Vec::new();
    for (path, interfaces) in object_manager.get_managed_objects().await? {
        if let Some(props) = interfaces.get("ca.kamacite.BootEnvironment") {
            let env = BootEnvironmentObject::from_properties(path, props)?;
            environments.push(env);
        }
    }

    // Sort by creation time.
    environments.sort_by(|a, b| a.created.cmp(&b.created));

    Ok(environments)
}

/// Activate a boot environment by its D-Bus object path using the provided connection
async fn activate_boot_environment(
    connection: &zbus::Connection,
    path: &zvariant::OwnedObjectPath,
) -> Result<(), zbus::Error> {
    // Create a proxy for this boot environment
    let proxy = BootEnvironmentProxy::builder(connection)
        .path(path)?
        .build()
        .await?;

    // Activate it temporarily.
    proxy.activate(true).await?;
    Ok(())
}

/// Create a COSMIC application from the app model
impl cosmic::Application for AppModel {
    /// The async executor that will be used to run your application's commands.
    type Executor = cosmic::SingleThreadExecutor;

    /// Data that your application receives to its init method.
    type Flags = ();

    /// Messages which the application and its widgets will emit.
    type Message = Message;

    /// Unique identifier in RDNN (reverse domain name notation) format.
    const APP_ID: &'static str = "ca.kamacite.CosmicBootEnvironmentApplet";

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
            conn: None,
        };

        // Spawn a task to open the D-Bus connection.
        let task = Task::perform(zbus::Connection::system(), |result| match result {
            Ok(conn) => cosmic::Action::App(Message::Connected(conn)),
            Err(e) => {
                tracing::error!(error = ?e, "Failed to connect to D-Bus");
                cosmic::Action::None
            }
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
            .on_press_down(Message::TogglePopup)
            .into()
    }

    fn view_window(&self, _id: Id) -> Element<'_, Self::Message> {
        let Spacing {
            space_xxs, space_s, ..
        } = theme::active().cosmic().spacing;

        // Build the column starting with boot environment rows
        let mut content = column![];

        // Display a summary of the active boot environment at the top.
        if let Some(active_env) = self.environments.iter().find(|e| e.active) {
            let title = if let Some(desc) = &active_env.description {
                // TODO: Add elipses to overlong descriptions.
                text::heading(format!("{} ({})", desc, active_env.name))
            } else {
                text::monotext(&active_env.name)
            };

            content = content.push(padded_control(
                row![
                    cosmic::widget::icon::from_name("drive-harddisk-system-symbolic").size(40),
                    column![title, text::caption(fl!("active-boot-env")),].width(Length::Fill),
                ]
                .align_y(Alignment::Center)
                .spacing(space_s),
            ));
        } else {
            content = content.push(padded_control(
                row![text::body(fl!("no-active-boot-env"))]
                    .align_y(Alignment::Center)
                    .spacing(space_s),
            ));
        }

        // Divider.
        content = content
            .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));

        // A dropdown for activating boot environments, if they exist.
        let dropdown_labels: Vec<String> = self
            .environments
            .iter()
            .map(|env| {
                // Build the label - either description or name
                if let Some(desc) = &env.description {
                    format!("{} ({})", desc, env.name)
                } else {
                    env.name.clone()
                }
            })
            .collect();

        if !dropdown_labels.is_empty() {
            // Don't distinguish between temporary and permanent activations.
            let selected_idx = self
                .environments
                .iter()
                .position(|e| e.boot_once)
                .or(self.environments.iter().position(|e| e.next_boot));

            let paths: Vec<zvariant::OwnedObjectPath> = self
                .environments
                .iter()
                .map(|env| env.path.clone())
                .collect();

            content = content.push(padded_control(
                row![
                    text::body(fl!("reboot-into")).width(Length::Fill),
                    dropdown(dropdown_labels, selected_idx, move |idx| {
                        Message::ActivateEnvironment(paths[idx].clone())
                    })
                ]
                .align_y(Alignment::Center)
                .spacing(space_s),
            ));

            // Divider.
            content = content
                .push(padded_control(divider::horizontal::default()).padding([space_xxs, space_s]));
        }

        // The "Boot settings..." button at the bottom that could open a
        // settings dialog.
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
        struct ObjectManagerSub;
        struct PropertiesChangedSub;

        if let Some(ref conn) = self.conn {
            Subscription::batch(vec![
                Subscription::run_with_id(
                    std::any::TypeId::of::<ObjectManagerSub>(),
                    object_manager_stream(conn.clone()),
                ),
                Subscription::run_with_id(
                    std::any::TypeId::of::<PropertiesChangedSub>(),
                    properties_changed_stream(conn.clone()),
                ),
            ])
        } else {
            Subscription::none()
        }
    }

    /// Handles messages emitted by the application and its widgets.
    ///
    /// Tasks may be returned for asynchronous execution of code in the background
    /// on the application's async runtime.
    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::BootSettingsClicked => {
                // Placeholder: would open boot settings configuration
                tracing::info!("Opening boot settings");
            }
            Message::Connected(conn) => {
                tracing::info!(
                    unique_name = conn
                        .unique_name()
                        .map(|name| name.to_string())
                        .unwrap_or_default(),
                    "Connected to system bus"
                );
                // Store the active connection and start a task to load existing
                // boot environments.
                self.conn = Some(conn.clone());
                return Task::perform(
                    async move { load_boot_environments(&conn).await },
                    |result| match result {
                        Ok(environments) => {
                            cosmic::Action::App(Message::BootEnvironmentsLoaded(environments))
                        }
                        Err(e) => {
                            tracing::error!(error = ?e, "Failed to load boot environments");
                            cosmic::Action::None
                        }
                    },
                );
            }
            Message::BootEnvironmentsLoaded(environments) => {
                tracing::info!(count = environments.len(), "Loaded boot environments");
                self.environments = environments;
            }
            Message::Added(env) => {
                tracing::info!(path = ?env.path, name = %env.name, "Boot environment added");
                // No need to re-sort, we know the new environment is the most recent.
                self.environments.push(env);
            }
            Message::Removed(path) => {
                tracing::info!(?path, "Boot environment removed");
                self.environments.retain(|env| env.path != path);
            }
            Message::BootEnvironmentsModified => {
                if let Some(conn) = self.conn.clone() {
                    return Task::perform(
                        async move { load_boot_environments(&conn).await },
                        |result| match result {
                            Ok(environments) => {
                                cosmic::Action::App(Message::BootEnvironmentsLoaded(environments))
                            }
                            Err(e) => {
                                tracing::error!(error = ?e, "Failed to reload boot environments");
                                cosmic::Action::None
                            }
                        },
                    );
                }
            }
            Message::ActivateEnvironment(path) => {
                if let Some(conn) = self.conn.clone() {
                    let path_ref = path.clone();
                    return Task::perform(
                        async move { activate_boot_environment(&conn, &path_ref).await },
                        move |result| {
                            match result {
                                Ok(()) => tracing::info!(
                                    path = path.to_string(),
                                    "Temporarily activated boot environment"
                                ),
                                Err(e) => {
                                    tracing::error!(path = path.to_string(), error = ?e, "Failed to activate boot environment")
                                }
                            };
                            cosmic::Action::None
                        },
                    );
                } else {
                    // It should never be possible to send this message without
                    // an active D-Bus connection.
                    unreachable!("no D-Bus connection available");
                }
            }
            Message::TogglePopup => {
                return if let Some(p) = self.popup.take() {
                    destroy_popup(p)
                } else {
                    let new_id = Id::unique();
                    self.popup.replace(new_id);
                    let popup_settings = self.core.applet.get_popup_settings(
                        self.core.main_window_id().unwrap(),
                        new_id,
                        None,
                        None,
                        None,
                    );
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

/// A stream of Added and Removed messages for the underlying boot environments.
fn object_manager_stream(
    conn: zbus::Connection,
) -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::stream::channel(32, move |mut channel| async move {
        let object_manager = match ObjectManagerProxy::builder(&conn)
            .destination("ca.kamacite.BootEnvironments1")
            // SAFETY: Safe to unwrap because the destination and path are known to be valid.
            .unwrap()
            .path("/ca/kamacite/BootEnvironments")
            .unwrap()
            .build()
            .await
        {
            Ok(proxy) => proxy,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to create ObjectManager proxy, updates will be ignored");
                return;
            }
        };

        let mut added_stream = match object_manager.receive_interfaces_added().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to start streaming InterfacesAdded signal");
                return;
            }
        };

        let mut removed_stream = match object_manager.receive_interfaces_removed().await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to start streaming InterfacesRemoved signal");
                return;
            }
        };

        loop {
            tokio::select! {
                Some(signal) = added_stream.next() => {
                    if let Ok(args) = signal.args() {
                        if let Some(props) = args.interfaces_and_properties.get("ca.kamacite.BootEnvironment") {
                            let path = From::from(args.object_path);
                            match BootEnvironmentObject::from_properties(path, props) {
                                Ok(env) => {
                                    // TODO: Should we log errors here?
                                    let _ = channel.send(Message::Added(env)).await;
                                }
                                Err(e) => {
                                    tracing::error!(error = ?e, "Failed to parse boot environment object");
                                }
                            }
                        }
                    }
                }
                Some(signal) = removed_stream.next() => {
                    if let Ok(args) = signal.args() {
                        let path = From::from(args.object_path);
                        // TODO: Should we log errors here?
                        let _ = channel.send(Message::Removed(path)).await;
                    }
                }
            }
        }
    })
}

/// A stream of PropertiesChanged messages for all boot environments.
fn properties_changed_stream(
    conn: zbus::Connection,
) -> impl cosmic::iced::futures::Stream<Item = Message> {
    cosmic::iced::stream::channel(32, move |mut channel| async move {
        // Match against all PropertiesChanged signals in the boot environment
        // namespace.
        let rule = match zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .interface("org.freedesktop.DBus.Properties")
            .and_then(|b| b.member("PropertiesChanged"))
            .and_then(|b| b.path_namespace("/ca/kamacite/BootEnvironments"))
        {
            Ok(builder) => builder.build(),
            Err(e) => {
                tracing::error!(error = ?e, "Failed to build match rule for PropertiesChanged");
                return;
            }
        };

        let mut stream = match zbus::MessageStream::for_match_rule(rule, &conn, Some(32)).await {
            Ok(stream) => stream,
            Err(e) => {
                tracing::error!(error = ?e, "Failed to start streaming PropertiesChanged signals");
                return;
            }
        };

        while let Some(msg_result) = stream.next().await {
            match msg_result {
                Ok(msg) => {
                    // We treat all property changes as triggering a reload.
                    // This isn't terribly efficient, but it does sidestep our
                    // getting out of sync with the backend by being too clever
                    // with our caching.
                    //
                    // Unfortunately, it also means that when multiple
                    // properties change -- which is common when a boot
                    // environment is activated -- we reload multiple times in
                    // succession.
                    let _ = channel.send(Message::BootEnvironmentsModified).await;

                    // We only need to parse the message for debug logs, so make
                    // this whole step conditional.
                    if tracing::enabled!(tracing::Level::DEBUG) {
                        match msg
                            .body()
                            .deserialize::<(String, HashMap<String, zvariant::Value<'_>>, Vec<String>)>()
                        {
                            Ok((iface, changed, _)) => {
                                let props: Vec<&str> = changed.keys().map(|s| s.as_str()).collect();
                                tracing::debug!(
                                    path = msg
                                        .header()
                                        .path()
                                        .map(|path| path.to_string())
                                        .unwrap_or_default(),
                                    iface,
                                    props = props.join(","),
                                    "One or more BootEnvironment properties updated"
                                );
                            }
                            Err(e) => {
                                tracing::error!(error = ?e, "Failed to parse PropertiesChanged signal");
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = ?e, "Error receiving PropertiesChanged signal");
                }
            }
        }
    })
}
