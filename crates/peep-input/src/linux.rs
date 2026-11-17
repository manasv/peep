use crate::Snapshot;
use anyhow::{Context as _, Result, bail};
use atspi::{
    ObjectRefOwned, Role, State,
    proxy::{accessible::AccessibleProxy, editable_text::EditableTextProxy, text::TextProxy},
};
use atspi_connection::AccessibilityConnection;
use std::{collections::VecDeque, time::Duration};
use tokio_util::sync::CancellationToken;
use zbus::{Connection, proxy::CacheProperties};

pub struct Target {
    object: ObjectRefOwned,
    window: ObjectRefOwned,
    snapshot: Snapshot,
    role: Role,
    pid: Option<u32>,
}

impl Target {
    pub fn belongs_to_process(&self, pid: u32) -> bool {
        self.pid == Some(pid)
    }
}

pub struct Context {
    runtime: tokio::runtime::Runtime,
    connection: Option<AccessibilityConnection>,
}
impl Context {
    pub fn new() -> Result<Self> {
        Ok(Self {
            runtime: tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?,
            connection: None,
        })
    }
    pub fn capture(&mut self) -> Result<Target> {
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                if self
                    .connection
                    .as_ref()
                    .is_some_and(|connection| connection.connection().is_closed())
                {
                    self.connection = None;
                }
                if self.connection.is_none() {
                    self.connection = Some(AccessibilityConnection::new().await?);
                }
                find_target(self.connection.as_ref().expect("accessibility connected")).await
            })
            .await
            .context("The editor did not respond to accessibility requests.")?
        })
    }
    pub fn insert(&mut self, target: Target, text: &str, cancel: &CancellationToken) -> Result<()> {
        self.runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                if text.trim().is_empty() || cancel.is_cancelled() {
                    bail!("Dictation canceled.");
                }
                let connection = self
                    .connection
                    .as_ref()
                    .context("Accessibility bus disconnected")?
                    .connection();
                let window = accessible(connection, &target.window).await?;
                if !window.get_state().await?.contains(State::Active) {
                    bail!("The original window is no longer focused.");
                }
                if snapshot(connection, &target.object).await? != target.snapshot {
                    bail!("The original field, cursor, or text changed.");
                }
                let expected = target.snapshot.replace(text)?;
                let editor = EditableTextProxy::builder(connection)
                    .destination(target.object.name_as_str().context("Missing editor")?)?
                    .path(target.object.path_as_str())?
                    .cache_properties(CacheProperties::No)
                    .build()
                    .await?;
                if cancel.is_cancelled() {
                    bail!("Dictation canceled.");
                }
                let inserted = if target.snapshot.start == target.snapshot.end {
                    editor
                        .insert_text(target.snapshot.start.try_into()?, text, text.len().try_into()?)
                        .await?
                } else if target.role == Role::Entry {
                    editor.set_text_contents(&expected).await?
                } else {
                    bail!("This editor cannot safely replace selected text. Clear the selection and try again.");
                };
                if !inserted {
                    bail!("The editor rejected text insertion.");
                }
                let text_proxy = text_proxy(connection, &target.object).await?;
                let _ = text_proxy
                    .set_caret_offset((target.snapshot.start + text.chars().count()).try_into()?)
                    .await;
                if text_proxy.get_text(0, -1).await? != expected {
                    bail!("The editor did not confirm the inserted text. Check the field before pasting manually.");
                }
                Ok(())
            })
            .await
            .context("The editor did not confirm delivery in time. Check the field before pasting manually.")?
        })
    }
    pub fn enable_accessibility(&mut self) -> Result<()> {
        self.runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(3),
                atspi_connection::set_session_accessibility(true),
            )
            .await??;
            Ok(())
        })
    }
    pub fn accessibility_status(&mut self) -> Result<bool> {
        self.runtime.block_on(async {
            Ok(tokio::time::timeout(
                Duration::from_secs(3),
                atspi_connection::read_session_accessibility(),
            )
            .await??)
        })
    }
}

async fn find_target(connection: &AccessibilityConnection) -> Result<Target> {
    let root = connection.root_accessible_on_registry().await?;
    let mut queue = VecDeque::new();
    for application in root.get_children().await? {
        queue.push_back((application, None, 0usize));
    }
    let mut visited = 0;
    while let Some((object, mut window, depth)) = queue.pop_front() {
        visited += 1;
        if visited > 4096 || depth > 64 {
            continue;
        }
        let Ok(accessible) = accessible(connection.connection(), &object).await else {
            continue;
        };
        let Ok(role) = accessible.get_role().await else {
            continue;
        };
        let Ok(states) = accessible.get_state().await else {
            continue;
        };
        if matches!(role, Role::Frame | Role::Dialog | Role::Window) {
            if !states.contains(State::Active) {
                continue;
            }
            window = Some(object.clone());
        }
        if matches!(role, Role::Terminal | Role::PasswordText) {
            continue;
        }
        if states.contains(State::Focused)
            && let Some(window) = window
        {
            let pid = connection
                .connection()
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetConnectionUnixProcessID",
                    &(object
                        .name_as_str()
                        .context("Missing application bus name")?,),
                )
                .await
                .and_then(|message| message.body().deserialize::<u32>());
            if pid.as_ref().is_ok_and(|pid| *pid == std::process::id()) {
                bail!("Choose a text field in another app.");
            }
            let snapshot = snapshot(connection.connection(), &object).await?;
            return Ok(Target {
                object,
                window,
                snapshot,
                role,
                pid: pid.ok(),
            });
        }
        if depth > 1 && !states.contains(State::Showing) {
            continue;
        }
        if let Ok(children) = accessible.get_children().await {
            for child in children {
                queue.push_back((child, window.clone(), depth + 1));
            }
        }
    }
    bail!(
        "No accessible focused editor. Enable Text insertion and the destination app's accessibility support."
    )
}

async fn accessible<'a>(
    connection: &'a Connection,
    object: &'a ObjectRefOwned,
) -> Result<AccessibleProxy<'a>> {
    Ok(AccessibleProxy::builder(connection)
        .destination(object.name_as_str().context("Missing editor bus name")?)?
        .path(object.path_as_str())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}
async fn text_proxy<'a>(
    connection: &'a Connection,
    object: &'a ObjectRefOwned,
) -> Result<TextProxy<'a>> {
    Ok(TextProxy::builder(connection)
        .destination(object.name_as_str().context("Missing editor bus name")?)?
        .path(object.path_as_str())?
        .cache_properties(CacheProperties::No)
        .build()
        .await?)
}
async fn snapshot(connection: &Connection, object: &ObjectRefOwned) -> Result<Snapshot> {
    let accessible = accessible(connection, object).await?;
    let role = accessible.get_role().await?;
    if matches!(role, Role::PasswordText | Role::Terminal) {
        bail!("Password fields and terminals are not used for dictation.");
    }
    let states = accessible.get_state().await?;
    if !states.contains(State::Focused)
        || !states.contains(State::Editable)
        || states.contains(State::Defunct)
    {
        bail!("Choose a focused editable text field.");
    }
    let proxy = text_proxy(connection, object).await?;
    let count = proxy.character_count().await?;
    if !(0..=1_000_000).contains(&count) {
        bail!("The editor is too large to verify safely.");
    }
    let text = proxy.get_text(0, count).await?;
    let selections = proxy.get_n_selections().await?;
    let (start, end) = match selections {
        0 => {
            let caret = proxy.caret_offset().await?;
            (caret, caret)
        }
        1 => proxy.get_selection(0).await?,
        _ => bail!("Multiple selections are not supported."),
    };
    let snapshot = Snapshot {
        text,
        start: start.try_into()?,
        end: end.try_into()?,
    };
    snapshot.replace("")?;
    Ok(snapshot)
}
