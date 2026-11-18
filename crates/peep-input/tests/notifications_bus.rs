#![cfg(all(unix, any(target_os = "linux", feature = "linux-integration")))]

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use zbus::zvariant::OwnedValue;

struct Notifications(Arc<Mutex<bool>>);
#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Notifications {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app: String,
        replaces: u32,
        _icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        hints: HashMap<String, OwnedValue>,
        timeout: i32,
    ) -> u32 {
        assert_eq!(app, "Peep");
        assert_eq!(replaces, 0);
        assert_eq!(summary, "Peep");
        assert_eq!(body, "Paste sent.");
        assert!(actions.is_empty());
        assert_eq!(u8::try_from(hints.get("urgency").unwrap()).unwrap(), 0);
        assert!(bool::try_from(hints.get("transient").unwrap()).unwrap());
        assert_eq!(timeout, 7000);
        *self.0.lock().unwrap() = true;
        1
    }
}

#[test]
fn notification_is_passive_transient_and_contains_only_the_status_message() {
    if std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        assert!(std::env::var_os("PEEP_REQUIRE_DBUS_TESTS").is_none());
        return;
    }
    async_io::block_on(async {
        let called = Arc::new(Mutex::new(false));
        let _server = zbus::connection::Builder::session()
            .unwrap()
            .name("org.freedesktop.Notifications")
            .unwrap()
            .serve_at(
                "/org/freedesktop/Notifications",
                Notifications(called.clone()),
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::Connection::session().await.unwrap();
        let (events, _) = async_channel::unbounded();
        peep_input::notifications::show(&client, "Paste sent.", None, &events)
            .await
            .unwrap();
        assert!(*called.lock().unwrap());
    });
}
