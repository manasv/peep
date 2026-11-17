#![cfg(target_os = "linux")]

use peep_input::NativeContext;
use std::{
    io::BufRead,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn native_gtk_editor_accepts_verified_unicode_delivery() {
    if std::env::var_os("PEEP_TEST_LINUX_EDITOR").is_none() {
        return;
    }
    let output = tempfile::tempdir().unwrap();
    let file = output.path().join("text.txt");
    let mut context = NativeContext::new().unwrap();
    context.enable_accessibility().unwrap();
    let source = r#"
import sys
import gi
gi.require_version('Gtk', '3.0')
from gi.repository import Gtk, GLib
GLib.set_prgname('peep-test')
window = Gtk.Window(title='Peep integration fixture')
window.set_default_size(500, 180)
entry = Gtk.Entry()
entry.set_text('a😀b')
def save(widget):
    with open(sys.argv[1], 'w', encoding='utf-8') as stream:
        stream.write(widget.get_text())
entry.connect('changed', save)
window.add(entry)
window.show_all()
window.present()
def focus():
    entry.grab_focus()
    entry.select_region(1, 2)
    save(entry)
    print('PEEP_FIXTURE_READY', flush=True)
    return False
GLib.timeout_add(300, focus)
Gtk.main()
"#;
    let mut fixture = Fixture(
        Command::new("/usr/bin/python3")
            .args(["-u", "-c", source])
            .arg(&file)
            .env("GTK_MODULES", "atk-bridge")
            .env("NO_AT_BRIDGE", "0")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let stdout = fixture.0.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if line == "PEEP_FIXTURE_READY" {
                let _ = tx.send(());
                break;
            }
        }
    });
    rx.recv_timeout(Duration::from_secs(10))
        .expect("GTK fixture startup");
    let deadline = Instant::now() + Duration::from_secs(10);
    let target = loop {
        match context.capture() {
            Ok(target) if target.belongs_to_process(fixture.0.id()) => break target,
            result => {
                assert!(
                    Instant::now() < deadline,
                    "GTK fixture did not become the accessible focused editor (found target: {})",
                    result.is_ok()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    };
    context
        .insert(target, "hello 🌍", &CancellationToken::new())
        .unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "ahello 🌍b");
}
