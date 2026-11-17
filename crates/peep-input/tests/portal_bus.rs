#![cfg(any(target_os = "linux", feature = "linux-integration"))]

use peep_input::portal::{self, Command};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use zbus::{
    Connection,
    message::Header,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};

type Dict = HashMap<String, OwnedValue>;
#[derive(Default)]
struct State {
    session: String,
    closed: usize,
    configured: usize,
    hold: bool,
    deny: bool,
    trigger: String,
}
type Shared = Arc<Mutex<State>>;
fn value(text: &str) -> OwnedValue {
    Value::from(text).try_to_owned().unwrap()
}
fn shortcut(label: &str) -> Vec<(String, Dict)> {
    vec![(
        "dictate".into(),
        HashMap::from([
            ("description".into(), value("Dictate")),
            ("trigger_description".into(), value(label)),
        ]),
    )]
}
struct Session {
    state: Shared,
    path: String,
}
#[zbus::interface(name = "org.freedesktop.portal.Session")]
impl Session {
    async fn close(&self, #[zbus(connection)] connection: &Connection) -> zbus::fdo::Result<()> {
        self.state.lock().unwrap().closed += 1;
        connection
            .emit_signal(
                None::<&str>,
                self.path.as_str(),
                "org.freedesktop.portal.Session",
                "Closed",
                &(),
            )
            .await
            .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))
    }
}
struct Portal(Shared);
fn path(header: &Header<'_>, kind: &str, options: &Dict, key: &str) -> OwnedObjectPath {
    let sender = header
        .sender()
        .unwrap()
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let token = <&str>::try_from(options.get(key).unwrap()).unwrap();
    format!("/org/freedesktop/portal/desktop/{kind}/{sender}/{token}")
        .try_into()
        .unwrap()
}
fn response(connection: Connection, path: OwnedObjectPath, code: u32, results: Dict) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        connection
            .emit_signal(
                None::<&str>,
                &path,
                "org.freedesktop.portal.Request",
                "Response",
                &(code, results),
            )
            .await
            .unwrap();
    });
}
#[zbus::interface(name = "org.freedesktop.portal.GlobalShortcuts")]
impl Portal {
    #[zbus(property)]
    fn version(&self) -> u32 {
        2
    }
    async fn create_session(
        &self,
        options: Dict,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let request = path(&header, "request", &options, "handle_token");
        let session = path(&header, "session", &options, "session_handle_token");
        self.0.lock().unwrap().session = session.as_str().into();
        connection
            .object_server()
            .at(
                session.clone(),
                Session {
                    state: self.0.clone(),
                    path: session.to_string(),
                },
            )
            .await
            .unwrap();
        response(
            connection.clone(),
            request.clone(),
            0,
            HashMap::from([("session_handle".into(), value(session.as_str()))]),
        );
        Ok(request)
    }
    async fn bind_shortcuts(
        &self,
        _session: OwnedObjectPath,
        shortcuts: Vec<(String, Dict)>,
        _parent: &str,
        options: Dict,
        #[zbus(header)] header: Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<OwnedObjectPath> {
        let request = path(&header, "request", &options, "handle_token");
        let (hold, deny) = {
            let mut state = self.0.lock().unwrap();
            state.trigger = <&str>::try_from(shortcuts[0].1.get("preferred_trigger").unwrap())
                .unwrap()
                .to_owned();
            (state.hold, state.deny)
        };
        if !hold {
            let values = Value::new(shortcut("Control+Shift+Space"))
                .try_to_owned()
                .unwrap();
            response(
                connection.clone(),
                request.clone(),
                if deny { 1 } else { 0 },
                HashMap::from([("shortcuts".into(), values)]),
            );
        }
        Ok(request)
    }
    async fn configure_shortcuts(
        &self,
        session: OwnedObjectPath,
        _parent: &str,
        _options: Dict,
        #[zbus(connection)] connection: &Connection,
    ) -> zbus::fdo::Result<()> {
        self.0.lock().unwrap().configured += 1;
        connection
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/portal/desktop",
                "org.freedesktop.portal.GlobalShortcuts",
                "ShortcutsChanged",
                &(session, shortcut("Alt+Space")),
            )
            .await
            .unwrap();
        Ok(())
    }
}
async fn receive(events: &async_channel::Receiver<peep_core::Event>) -> peep_core::Event {
    tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("Portal event timeout")
        .unwrap()
}
async fn activate(connection: &Connection, session: &str) {
    connection
        .emit_signal(
            None::<&str>,
            "/org/freedesktop/portal/desktop",
            "org.freedesktop.portal.GlobalShortcuts",
            "Activated",
            &(
                OwnedObjectPath::try_from(session).unwrap(),
                "dictate",
                1u64,
                Dict::new(),
            ),
        )
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn portal_binds_filters_configures_revokes_and_cancels_pending_permission() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(
            std::env::var_os("PEEP_REQUIRE_DBUS_TESTS").is_none(),
            "Run under dbus-run-session"
        );
        return;
    }
    let state = Shared::default();
    let connection = zbus::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.portal.Desktop")
        .unwrap()
        .serve_at("/org/freedesktop/portal/desktop", Portal(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let (events_tx, events) = async_channel::unbounded();
    let (commands, done) = portal::spawn(events_tx);
    commands
        .send(Command::Bind {
            generation: 1,
            trigger: Some("CTRL+SHIFT+space".into()),
        })
        .await
        .unwrap();
    let ready = receive(&events).await;
    assert_eq!(ready.kind, "portal_shortcut_ready", "{}", ready.text);
    assert_eq!(ready.id, 1);
    assert_eq!(state.lock().unwrap().trigger, "CTRL+SHIFT+space");
    let old_session = state.lock().unwrap().session.clone();
    activate(&connection, "/wrong/session").await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
    activate(&connection, &old_session).await;
    assert_eq!(receive(&events).await.kind, "portal_hotkey");
    commands.send(Command::Configure).await.unwrap();
    assert_eq!(receive(&events).await.text, "Alt+Space");
    assert_eq!(state.lock().unwrap().configured, 1);
    commands
        .send(Command::Bind {
            generation: 2,
            trigger: Some("ALT+space".into()),
        })
        .await
        .unwrap();
    assert_eq!(receive(&events).await.id, 2);
    assert_eq!(state.lock().unwrap().closed, 1);
    activate(&connection, &old_session).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), events.recv())
            .await
            .is_err()
    );
    let session = state.lock().unwrap().session.clone();
    connection
        .emit_signal(
            None::<&str>,
            session.as_str(),
            "org.freedesktop.portal.Session",
            "Closed",
            &(),
        )
        .await
        .unwrap();
    assert_eq!(receive(&events).await.kind, "portal_shortcut_failed");
    state.lock().unwrap().deny = true;
    commands
        .send(Command::Bind {
            generation: 3,
            trigger: Some("ALT+space".into()),
        })
        .await
        .unwrap();
    let denied = receive(&events).await;
    assert_eq!(denied.kind, "portal_shortcut_failed");
    assert_eq!(denied.id, 3);
    state.lock().unwrap().hold = true;
    commands
        .send(Command::Bind {
            generation: 4,
            trigger: Some("ALT+space".into()),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if state.lock().unwrap().session != session {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    commands.send(Command::Shutdown).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), done.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.lock().unwrap().closed, 4);
    let (control_events, actions) = async_channel::unbounded();
    let (shutdown, control_done) = peep_input::control::spawn(control_events);
    assert_eq!(receive(&actions).await.kind, "control_ready");
    for (command, kind) in [
        ("--toggle", "toggle_dictation"),
        ("--stop", "stop"),
        ("--cancel", "cancel"),
        ("--open", "open"),
    ] {
        tokio::task::spawn_blocking(move || peep_input::control::send(command))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(receive(&actions).await.kind, kind);
    }
    shutdown.send(()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), control_done.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(
        tokio::task::spawn_blocking(|| peep_input::control::send("--toggle"))
            .await
            .unwrap()
            .is_err()
    );
}
