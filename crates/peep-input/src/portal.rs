use anyhow::{Context as _, Result, bail};
use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
use futures_util::StreamExt;
use peep_core::Event;
use std::{collections::HashMap, time::Duration};

pub enum Command {
    Bind {
        generation: u64,
        trigger: Option<String>,
    },
    Configure,
    Shutdown,
}

pub fn spawn(
    events: async_channel::Sender<Event>,
) -> (async_channel::Sender<Command>, async_channel::Receiver<()>) {
    let (tx, rx) = async_channel::unbounded();
    let (done_tx, done) = async_channel::bounded(1);
    std::thread::Builder::new()
        .name("peep-shortcut-portal".into())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("portal runtime");
            runtime.block_on(async {
                let mut pending = None;
                loop {
                    let command = match pending.take() {
                        Some(command) => command,
                        None => match rx.recv().await {
                            Ok(command) => command,
                            Err(_) => break,
                        },
                    };
                    match command {
                        Command::Bind {
                            generation,
                            trigger: Some(trigger),
                        } => match bind(generation, &trigger, &events, &rx).await {
                            Ok(command) => pending = Some(command),
                            Err(error) => {
                                let _ = events.try_send(Event {
                                    kind: "portal_shortcut_failed".into(),
                                    id: generation,
                                    text: format!("Global Shortcuts portal: {error}"),
                                    ..Default::default()
                                });
                            }
                        },
                        Command::Shutdown => break,
                        _ => {}
                    }
                }
            });
            let _ = done_tx.try_send(());
        })
        .expect("start shortcut portal worker");
    (tx, done)
}

async fn bind(
    generation: u64,
    trigger: &str,
    events: &async_channel::Sender<Event>,
    commands: &async_channel::Receiver<Command>,
) -> Result<Command> {
    let _ = tokio::time::timeout(
        Duration::from_secs(3),
        ashpd::register_host_app("dev.peep.app".parse()?),
    )
    .await;
    let portal = tokio::time::timeout(Duration::from_secs(5), GlobalShortcuts::new()).await??;
    let session = tokio::time::timeout(Duration::from_secs(5), portal.create_session()).await??;
    let session_path = serde_json::to_value(&session)?
        .as_str()
        .context("Invalid portal session path")?
        .to_owned();
    let result = async {
        let mut activated = portal.receive_activated().await?;
        let mut changed = portal.receive_shortcuts_changed().await?;
        let mut closed = session.receive_closed().await?;
        let shortcuts = [NewShortcut::new("dictate", "Start or finish dictation")
            .preferred_trigger(trigger)];
        let binding = portal.bind_shortcuts(&session, &shortcuts, None);
        tokio::pin!(binding);
        let response = tokio::select! {
            response = &mut binding => response?.response()?,
            command = commands.recv() => return Ok(command.unwrap_or(Command::Shutdown)),
        };
        let shortcut = response
            .shortcuts()
            .iter()
            .find(|shortcut| shortcut.id() == "dictate")
            .context("Shortcut permission was not granted")?;
        ready(events, generation, shortcut.trigger_description());
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    Ok(Command::Configure) => {
                        let options: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::new();
                        let arguments = (&session, "", options);
                        let request = portal.call::<_, _, ()>("ConfigureShortcuts", &arguments);
                        if let Err(error) = tokio::time::timeout(Duration::from_secs(5), request)
                            .await.context("Shortcut settings did not respond")
                            .and_then(|result| result.map_err(Into::into))
                        {
                            let _ = events.try_send(Event {
                                kind: "shortcut_rejected".into(),
                                text: format!("Configure this shortcut in your desktop settings (portal version 2 is required): {error}"),
                                ..Default::default()
                            });
                        }
                    }
                    Ok(command) => return Ok(command),
                    Err(_) => return Ok(Command::Shutdown),
                },
                event = activated.next() => {
                    let Some(event) = event else {
                        bail!("Shortcut portal disconnected");
                    };
                    if event.shortcut_id() == "dictate" && event.session_handle().as_str() == session_path {
                        let _ = events.try_send(Event {
                            kind: "portal_hotkey".into(),
                            id: generation,
                            ..Default::default()
                        });
                    }
                }
                event = changed.next() => {
                    let Some(event) = event else {
                        bail!("Shortcut portal disconnected");
                    };
                    if event.session_handle().as_str() != session_path {
                        continue;
                    }
                    if let Some(shortcut) = event.shortcuts().iter().find(|shortcut| shortcut.id() == "dictate") {
                        ready(events, generation, shortcut.trigger_description());
                    } else {
                        bail!("The desktop revoked the shortcut binding");
                    }
                }
                _ = closed.next() => bail!("Shortcut session closed by the desktop"),
            }
        }
    }
    .await;
    let _ = tokio::time::timeout(Duration::from_secs(5), session.close()).await;
    result
}

fn ready(events: &async_channel::Sender<Event>, generation: u64, label: &str) {
    let _ = events.try_send(Event {
        kind: "portal_shortcut_ready".into(),
        id: generation,
        text: label.to_owned(),
        ..Default::default()
    });
}

pub fn trigger(chord: &str) -> Result<String> {
    let mut parts = Vec::new();
    for part in chord.split('+') {
        parts.push(match part {
            "Control" => "CTRL".to_owned(),
            "Alt" => "ALT".to_owned(),
            "Super" | "Meta" => "LOGO".to_owned(),
            "Shift" => "SHIFT".to_owned(),
            "Space" => "space".to_owned(),
            "Enter" => "Return".to_owned(),
            "Tab" => "Tab".to_owned(),
            key if key.starts_with("Key") && key.len() == 4 => key[3..].to_lowercase(),
            key if key.starts_with("Digit") && key.len() == 6 => key[5..].to_owned(),
            _ => bail!("Unsupported shortcut key"),
        });
    }
    Ok(parts.join("+"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portal_triggers_follow_the_xdg_shortcuts_syntax() {
        assert_eq!(trigger("Control+Shift+Space").unwrap(), "CTRL+SHIFT+space");
        assert_eq!(trigger("Alt+Super+KeyK").unwrap(), "ALT+LOGO+k");
        assert!(trigger("Control+Unsupported").is_err());
    }
}
