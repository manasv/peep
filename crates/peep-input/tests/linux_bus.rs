#![cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]

use atspi::{ObjectRef, ObjectRefOwned, Role, State, StateSet};
use peep_input::linux::Context;
use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

#[test]
fn dbus_connects_without_a_tokio_runtime() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(std::env::var_os("PEEP_REQUIRE_DBUS_TESTS").is_none());
        return;
    }
    async_io::block_on(async {
        let connection = zbus::Connection::session().await.unwrap();
        let bus = zbus::fdo::DBusProxy::new(&connection).await.unwrap();
        assert!(!bus.get_id().await.unwrap().is_empty());
        connection.close().await.unwrap();
    });
}

#[derive(Default)]
struct StateData {
    text: String,
    start: i32,
    end: i32,
    focused: bool,
    active: bool,
    editable: bool,
    password: bool,
    enabled: bool,
    writes: u32,
}
type Shared = Arc<Mutex<StateData>>;
struct Accessible {
    kind: u8,
    name: String,
    state: Shared,
}
fn object(name: &str, kind: u8) -> ObjectRefOwned {
    ObjectRef::new_owned(
        zbus::names::UniqueName::try_from(name.to_owned()).unwrap(),
        zbus::zvariant::ObjectPath::try_from(match kind {
            0 => "/org/a11y/atspi/accessible/root",
            1 => "/frame",
            _ => "/field",
        })
        .unwrap(),
    )
}
#[zbus::interface(name = "org.a11y.atspi.Accessible")]
impl Accessible {
    fn get_interfaces(&self) -> Vec<String> {
        let mut interfaces = vec!["org.a11y.atspi.Accessible".into()];
        if self.kind == 2 {
            interfaces.extend([
                "org.a11y.atspi.Text".into(),
                "org.a11y.atspi.EditableText".into(),
            ]);
        }
        interfaces
    }
    fn get_children(&self) -> Vec<ObjectRefOwned> {
        if self.kind < 2 {
            vec![object(&self.name, self.kind + 1)]
        } else {
            vec![]
        }
    }
    fn get_role(&self) -> Role {
        match self.kind {
            0 => Role::Application,
            1 => Role::Frame,
            _ if self.state.lock().unwrap().password => Role::PasswordText,
            _ => Role::Entry,
        }
    }
    fn get_state(&self) -> StateSet {
        let state = self.state.lock().unwrap();
        let mut result = StateSet::empty();
        result.insert(State::Showing);
        if self.kind == 1 && state.active {
            result.insert(State::Active);
        }
        if self.kind == 2 && state.focused {
            result.insert(State::Focused);
        }
        if self.kind == 2 && state.editable {
            result.insert(State::Editable);
        }
        result
    }
}
struct Text(Shared);
#[zbus::interface(name = "org.a11y.atspi.Text")]
impl Text {
    fn get_text(&self, start: i32, end: i32) -> String {
        let state = self.0.lock().unwrap();
        state
            .text
            .chars()
            .skip(start as usize)
            .take(if end < 0 {
                usize::MAX
            } else {
                (end - start) as usize
            })
            .collect()
    }
    fn get_n_selections(&self) -> i32 {
        let state = self.0.lock().unwrap();
        i32::from(state.start != state.end)
    }
    fn get_selection(&self, _selection: i32) -> (i32, i32) {
        let state = self.0.lock().unwrap();
        (state.start, state.end)
    }
    fn set_caret_offset(&self, offset: i32) -> bool {
        let mut state = self.0.lock().unwrap();
        state.start = offset;
        state.end = offset;
        true
    }
    #[zbus(property)]
    fn character_count(&self) -> i32 {
        self.0.lock().unwrap().text.chars().count() as i32
    }
    #[zbus(property)]
    fn caret_offset(&self) -> i32 {
        self.0.lock().unwrap().end
    }
}
struct Editable(Shared);
#[zbus::interface(name = "org.a11y.atspi.EditableText")]
impl Editable {
    fn insert_text(&self, position: i32, text: &str, length: i32) -> bool {
        if usize::try_from(length).ok() != Some(text.len()) {
            return false;
        }
        let mut state = self.0.lock().unwrap();
        let snapshot = peep_input::Snapshot {
            text: state.text.clone(),
            start: position as usize,
            end: position as usize,
        };
        state.text = snapshot.replace(text).unwrap();
        state.writes += 1;
        true
    }
    fn set_text_contents(&self, text: &str) -> bool {
        let mut state = self.0.lock().unwrap();
        state.text = text.into();
        state.writes += 1;
        true
    }
}
struct Bus;
#[zbus::interface(name = "org.a11y.Bus")]
impl Bus {
    fn get_address(&self) -> String {
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    }
}
struct Status(Shared);
#[zbus::interface(name = "org.a11y.Status")]
impl Status {
    #[zbus(property)]
    fn is_enabled(&self) -> bool {
        self.0.lock().unwrap().enabled
    }
    #[zbus(property)]
    fn set_is_enabled(&mut self, enabled: bool) {
        self.0.lock().unwrap().enabled = enabled;
    }
}
struct Control(Shared);
#[zbus::interface(name = "dev.peep.TestEditor")]
impl Control {
    #[allow(clippy::too_many_arguments)]
    fn reset(
        &self,
        text: &str,
        start: i32,
        end: i32,
        focused: bool,
        active: bool,
        editable: bool,
        password: bool,
    ) {
        let mut state = self.0.lock().unwrap();
        *state = StateData {
            text: text.into(),
            start,
            end,
            focused,
            active,
            editable,
            password,
            ..Default::default()
        };
    }
    fn read(&self) -> (String, u32) {
        let state = self.0.lock().unwrap();
        (state.text.clone(), state.writes)
    }
}

#[test]
fn fixture_server() {
    if std::env::var_os("PEEP_BUS_FIXTURE").is_none() {
        return;
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let state = Shared::default();
        let connection = zbus::connection::Builder::session()
            .unwrap()
            .name("org.a11y.Bus")
            .unwrap()
            .name("org.a11y.atspi.Registry")
            .unwrap()
            .name("dev.peep.TestEditor")
            .unwrap()
            .build()
            .await
            .unwrap();
        let name = connection.unique_name().unwrap().to_string();
        for kind in 0..=2 {
            let reference = object(&name, kind);
            connection
                .object_server()
                .at(
                    reference.path_as_str(),
                    Accessible {
                        kind,
                        name: name.clone(),
                        state: state.clone(),
                    },
                )
                .await
                .unwrap();
        }
        connection
            .object_server()
            .at("/field", Text(state.clone()))
            .await
            .unwrap();
        connection
            .object_server()
            .at("/field", Editable(state.clone()))
            .await
            .unwrap();
        connection
            .object_server()
            .at("/org/a11y/bus", Bus)
            .await
            .unwrap();
        connection
            .object_server()
            .at("/org/a11y/bus", Status(state.clone()))
            .await
            .unwrap();
        connection
            .object_server()
            .at("/control", Control(state))
            .await
            .unwrap();
        println!("PEEP_FIXTURE_READY");
        std::io::stdout().flush().unwrap();
        std::future::pending::<()>().await;
    });
}
struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn accessible_delivery_checks_focus_selection_text_privacy_and_cancellation() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(
            std::env::var_os("PEEP_REQUIRE_DBUS_TESTS").is_none(),
            "Run under dbus-run-session"
        );
        return;
    }
    let mut fixture = Fixture(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fixture_server", "--nocapture"])
            .env("PEEP_BUS_FIXTURE", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = fixture.0.stdout.take().unwrap();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if line == "PEEP_FIXTURE_READY" {
                let _ = ready_tx.send(());
                break;
            }
        }
    });
    ready_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Fixture startup");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let connection = runtime.block_on(zbus::Connection::session()).unwrap();
    let control = runtime
        .block_on(zbus::Proxy::new(
            &connection,
            "dev.peep.TestEditor",
            "/control",
            "dev.peep.TestEditor",
        ))
        .unwrap();
    let reset =
        |start: i32, end: i32, focused: bool, active: bool, editable: bool, password: bool| {
            runtime
                .block_on(control.call::<_, _, ()>(
                    "Reset",
                    &("a😀b", start, end, focused, active, editable, password),
                ))
                .unwrap();
        };
    let read = || {
        runtime
            .block_on(control.call::<_, _, (String, u32)>("Read", &()))
            .unwrap()
    };
    let mut context = Context::new().unwrap();
    reset(1, 2, true, true, true, false);
    let target = context.capture().unwrap();
    context
        .insert(target, "hello", &CancellationToken::new())
        .unwrap();
    assert_eq!(read(), ("ahellob".into(), 1));
    reset(1, 1, true, true, true, false);
    let target = context.capture().unwrap();
    context
        .insert(target, "text🌍", &CancellationToken::new())
        .unwrap();
    assert_eq!(read(), ("atext🌍😀b".into(), 1));
    for (focused, active, start, end) in
        [(false, true, 1, 1), (true, false, 1, 1), (true, true, 2, 2)]
    {
        reset(1, 1, true, true, true, false);
        let target = context.capture().unwrap();
        reset(start, end, focused, active, true, false);
        assert!(
            context
                .insert(target, "wrong", &CancellationToken::new())
                .is_err()
        );
        assert_eq!(read(), ("a😀b".into(), 0));
    }
    for (editable, password) in [(false, false), (true, true)] {
        reset(1, 1, true, true, editable, password);
        assert!(context.capture().is_err());
        assert_eq!(read().1, 0);
    }
    reset(1, 1, true, true, true, false);
    let target = context.capture().unwrap();
    let canceled = CancellationToken::new();
    canceled.cancel();
    assert!(context.insert(target, "wrong", &canceled).is_err());
    assert_eq!(read().1, 0);
    assert!(!context.accessibility_status().unwrap());
    context.enable_accessibility().unwrap();
    assert!(context.accessibility_status().unwrap());
}
