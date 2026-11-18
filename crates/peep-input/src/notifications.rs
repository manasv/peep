use anyhow::Result;
use futures_util::StreamExt;
use peep_core::Event;
use std::{collections::HashMap, time::Duration};
use zbus::{Connection, Proxy, zvariant::Value};

pub fn notify(text: String, recovery: Option<String>, events: async_channel::Sender<Event>) {
    let _ = std::thread::Builder::new()
        .name("peep-notification".into())
        .spawn(move || {
            let result = (|| -> Result<()> {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                runtime.block_on(async {
                    let connection = Connection::session().await?;
                    show(&connection, &text, recovery, &events).await
                })
            })();
            if let Err(error) = result {
                eprintln!("Peep notification could not be shown: {error}");
            }
        });
}

pub async fn show(
    connection: &Connection,
    text: &str,
    recovery: Option<String>,
    events: &async_channel::Sender<Event>,
) -> Result<()> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await?;
    let mut actions = proxy.receive_signal("ActionInvoked").await?;
    let buttons = if recovery.is_some() {
        vec!["copy", "Copy text", "discard", "Discard"]
    } else {
        vec![]
    };
    let hints = HashMap::from([
        ("urgency", Value::U8(0)),
        ("transient", Value::Bool(true)),
        ("desktop-entry", Value::from("dev.peep.app")),
    ]);
    let id: u32 = proxy
        .call(
            "Notify",
            &(
                "Peep",
                0u32,
                "dev.peep.app",
                "Peep",
                text,
                buttons,
                hints,
                7000i32,
            ),
        )
        .await?;
    if let Some(text) = recovery {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        while let Ok(Some(message)) = tokio::time::timeout_at(deadline, actions.next()).await {
            let (notification, action): (u32, String) = message.body().deserialize()?;
            if notification == id && matches!(action.as_str(), "copy" | "discard") {
                let _ = events.try_send(Event {
                    kind: format!("toast_{action}"),
                    text,
                    ..Default::default()
                });
                break;
            }
        }
    }
    Ok(())
}
