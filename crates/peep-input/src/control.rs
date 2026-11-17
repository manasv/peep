use anyhow::{Result, bail};
use peep_core::Event;
use std::time::Duration;

struct Control(async_channel::Sender<Event>);
#[zbus::interface(name = "dev.peep.Control")]
impl Control {
    fn toggle(&self) {
        self.emit("toggle_dictation");
    }
    fn stop(&self) {
        self.emit("stop");
    }
    fn cancel(&self) {
        self.emit("cancel");
    }
    fn open(&self) {
        self.emit("open");
    }
}
impl Control {
    fn emit(&self, kind: &str) {
        let _ = self.0.try_send(Event {
            kind: kind.into(),
            ..Default::default()
        });
    }
}

pub fn spawn(
    events: async_channel::Sender<Event>,
) -> (async_channel::Sender<()>, async_channel::Receiver<()>) {
    let (shutdown, stop) = async_channel::bounded(1);
    let (done_tx, done) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-desktop-control".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("desktop control runtime");
            runtime.block_on(async {
                let connection = tokio::time::timeout(Duration::from_secs(3), async {
                    zbus::connection::Builder::session()?
                        .name("dev.peep.app")?
                        .serve_at("/dev/peep/app", Control(events.clone()))?
                        .build()
                        .await
                })
                .await;
                match connection {
                    Ok(Ok(connection)) => {
                        let _ = events.try_send(Event {
                            kind: "control_ready".into(),
                            ..Default::default()
                        });
                        let _ = stop.recv().await;
                        let _ = connection.close().await;
                    }
                    error => {
                        let _ = events.try_send(Event {
                            kind: "notice".into(),
                            text: format!(
                                "Compositor shortcut commands are unavailable: {error:?}"
                            ),
                            ..Default::default()
                        });
                    }
                }
            });
            let _ = done_tx.try_send(());
        })
        .expect("start desktop control");
    (shutdown, done)
}

pub fn send(action: &str) -> Result<()> {
    let method = match action {
        "--toggle" => "Toggle",
        "--stop" => "Stop",
        "--cancel" => "Cancel",
        "--open" => "Open",
        _ => bail!("Unknown desktop control command"),
    };
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?
        .block_on(async {
            tokio::time::timeout(Duration::from_secs(3), async {
                let connection = zbus::Connection::session().await?;
                let proxy = zbus::Proxy::new(
                    &connection,
                    "dev.peep.app",
                    "/dev/peep/app",
                    "dev.peep.Control",
                )
                .await?;
                proxy.call::<_, _, ()>(method, &()).await?;
                Ok::<_, anyhow::Error>(())
            })
            .await?
        })
}
