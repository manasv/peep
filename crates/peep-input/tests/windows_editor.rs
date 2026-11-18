#![cfg(target_os = "windows")]

use peep_input::NativeContext;
use std::{
    io::{BufRead, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
use windows::{
    Win32::{
        Foundation::{ERROR_SUCCESS, GetLastError, HWND, LPARAM, LRESULT, SetLastError, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::{Controls::*, Input::KeyboardAndMouse::SetFocus, WindowsAndMessaging::*},
    },
    core::{HSTRING, w},
};

const SET_PASSWORD_CHAR: u32 = WM_APP + 3;

unsafe extern "system" fn fixture_window_proc(
    window: HWND,
    code: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if code == SET_PASSWORD_CHAR {
            let edit = HWND(GetWindowLongPtrW(window, GWLP_USERDATA) as *mut _);
            return SendMessageW(edit, EM_SETPASSWORDCHAR, Some(wparam), Some(lparam));
        }
        DefWindowProcW(window, code, wparam, lparam)
    }
}

#[test]
fn fixture_server() {
    if std::env::var_os("PEEP_WINDOWS_FIXTURE").is_none() {
        return;
    }
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = w!("PeepEditorFixture");
        assert_ne!(
            RegisterClassW(&WNDCLASSW {
                lpfnWndProc: Some(fixture_window_proc),
                hInstance: instance.into(),
                lpszClassName: class,
                ..Default::default()
            }),
            0,
        );
        let frame = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class,
            w!("Peep integration fixture"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            60,
            60,
            600,
            300,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!("a😀b"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            10,
            10,
            500,
            100,
            Some(frame),
            None,
            None,
            None,
        )
        .unwrap();
        let other = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!("other"),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            10,
            130,
            500,
            50,
            Some(frame),
            None,
            None,
            None,
        )
        .unwrap();
        SetWindowLongPtrW(frame, GWLP_USERDATA, edit.0 as isize);
        let _ = SetForegroundWindow(frame);
        let _ = SetFocus(Some(edit));
        SendMessageW(edit, EM_SETSEL, Some(WPARAM(1)), Some(LPARAM(3)));
        println!(
            "PEEP_FIXTURE_READY {} {} {}",
            frame.0 as usize, edit.0 as usize, other.0 as usize
        );
        std::io::stdout().flush().unwrap();
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            if message.message == WM_APP + 1 {
                let _ = SetForegroundWindow(frame);
                let _ = SetFocus(Some(edit));
            }
            if message.message == WM_APP + 2 {
                let _ = SetFocus(Some(other));
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
struct Fixture(Child);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[track_caller]
fn message(window: HWND, code: u32, wparam: WPARAM, lparam: LPARAM) -> usize {
    let mut result = 0;
    unsafe { SetLastError(ERROR_SUCCESS) };
    assert_ne!(
        unsafe {
            SendMessageTimeoutW(
                window,
                code,
                wparam,
                lparam,
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                2_000,
                Some(&mut result),
            )
        }
        .0,
        0,
        "Fixture message 0x{code:04X} to {window:?} failed: {:?}; window exists: {}",
        unsafe { GetLastError() },
        unsafe { IsWindow(Some(window)).as_bool() },
    );
    result
}
fn contents(window: HWND) -> String {
    let length = message(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0));
    let mut buffer = vec![0u16; length + 1];
    let count = message(
        window,
        WM_GETTEXT,
        WPARAM(buffer.len()),
        LPARAM(buffer.as_mut_ptr() as isize),
    );
    String::from_utf16(&buffer[..count]).unwrap()
}

#[test]
fn native_windows_delivery_preserves_text_and_rejects_changed_or_protected_fields() {
    if std::env::var_os("PEEP_TEST_WINDOWS_UI").is_none() {
        return;
    }
    let mut fixture = Fixture(
        Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "fixture_server", "--nocapture"])
            .env("PEEP_WINDOWS_FIXTURE", "1")
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
            if line.starts_with("PEEP_FIXTURE_READY ") {
                let handles: Vec<usize> = line
                    .split_whitespace()
                    .skip(1)
                    .map(|s| s.parse().unwrap())
                    .collect();
                let _ = tx.send(handles);
            } else {
                eprintln!("Windows fixture: {line}");
            }
        }
    });
    let handles = rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(
        fixture.0.try_wait().unwrap().is_none(),
        "Fixture exited before setup"
    );
    let (frame, edit, other) = (
        HWND(handles[0] as *mut _),
        HWND(handles[1] as *mut _),
        HWND(handles[2] as *mut _),
    );
    let focus = || {
        unsafe {
            let _ = AllowSetForegroundWindow(fixture.0.id());
            PostMessageW(Some(frame), WM_APP + 1, WPARAM(0), LPARAM(0)).unwrap();
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while unsafe { GetForegroundWindow() } != frame && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            unsafe { GetForegroundWindow() },
            frame,
            "An interactive Windows desktop is required"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let reset = || {
        message(edit, EM_SETREADONLY, WPARAM(0), LPARAM(0));
        message(frame, SET_PASSWORD_CHAR, WPARAM(0), LPARAM(0));
        let value = HSTRING::from("a😀b");
        message(edit, WM_SETTEXT, WPARAM(0), LPARAM(value.as_ptr() as isize));
        message(edit, EM_SETSEL, WPARAM(1), LPARAM(3));
        focus();
    };
    let mut context = NativeContext::new().unwrap();
    message(frame, WM_NULL, WPARAM(0), LPARAM(0));
    reset();
    unsafe {
        let toast = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("Peep notification"),
            WS_POPUP,
            100,
            100,
            380,
            128,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        peep_input::show_toast_window(toast.0 as isize, 100., 100., 380., 128.).unwrap();
        assert!(IsWindowVisible(toast).as_bool());
        assert_eq!(
            GetForegroundWindow(),
            frame,
            "A notification must not take focus from the editor"
        );
        assert_ne!(
            GetWindowLongPtrW(toast, GWL_EXSTYLE) & WS_EX_NOACTIVATE.0 as isize,
            0
        );
        DestroyWindow(toast).unwrap();
    }
    let target = context.capture().unwrap();
    assert!(target.belongs_to_process(fixture.0.id()));
    context
        .insert(target, "hello 🌍", &CancellationToken::new())
        .unwrap();
    assert_eq!(contents(edit), "ahello 🌍b");
    reset();
    let target = context.capture().unwrap();
    assert!(target.belongs_to_process(fixture.0.id()));
    message(edit, EM_SETSEL, WPARAM(0), LPARAM(0));
    assert!(
        context
            .insert(target, "wrong", &CancellationToken::new())
            .is_err()
    );
    assert_eq!(contents(edit), "a😀b");
    reset();
    let target = context.capture().unwrap();
    assert!(target.belongs_to_process(fixture.0.id()));
    let changed = HSTRING::from("changed");
    message(
        edit,
        WM_SETTEXT,
        WPARAM(0),
        LPARAM(changed.as_ptr() as isize),
    );
    assert!(
        context
            .insert(target, "wrong", &CancellationToken::new())
            .is_err()
    );
    assert_eq!(contents(edit), "changed");
    reset();
    let target = context.capture().unwrap();
    assert!(target.belongs_to_process(fixture.0.id()));
    unsafe {
        PostMessageW(Some(frame), WM_APP + 2, WPARAM(0), LPARAM(0)).unwrap();
    }
    std::thread::sleep(Duration::from_millis(50));
    assert!(
        context
            .insert(target, "wrong", &CancellationToken::new())
            .is_err()
    );
    assert_eq!(contents(edit), "a😀b");
    assert_eq!(contents(other), "other");
    reset();
    message(edit, EM_SETREADONLY, WPARAM(1), LPARAM(0));
    assert!(context.capture().is_err());
    reset();
    message(frame, SET_PASSWORD_CHAR, WPARAM(42), LPARAM(0));
    assert!(context.capture().is_err());
    reset();
    let target = context.capture().unwrap();
    assert!(target.belongs_to_process(fixture.0.id()));
    let canceled = CancellationToken::new();
    canceled.cancel();
    assert!(context.insert(target, "wrong", &canceled).is_err());
    assert_eq!(contents(edit), "a😀b");
}
