//! Windows keyboard transport. One Raw Input consumer, no keyboard hook and
//! no DOM counting; the mouse retains its existing listen-only hook.
use super::*;
use std::cell::RefCell;
use windows_sys::Win32::{
    Foundation::{GetLastError, HINSTANCE, HWND},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{Input::*, WindowsAndMessaging::*},
};

thread_local! {
    static PIPELINE: RefCell<Option<WindowsInputPipeline>> = const { RefCell::new(None) };
}

unsafe extern "system" fn mouse_hook(code: i32, message: usize, data: isize) -> isize {
    if code == HC_ACTION as i32 {
        let event = match message as u32 {
            WM_LBUTTONDOWN => WindowsCandidateEvent::LeftMouseDown,
            WM_RBUTTONDOWN => WindowsCandidateEvent::RightMouseDown,
            WM_MBUTTONDOWN => WindowsCandidateEvent::MiddleMouseDown,
            _ => WindowsCandidateEvent::Ignored,
        };
        PIPELINE.with(|pipeline| {
            if let Ok(mut pipeline) = pipeline.try_borrow_mut()
                && let Some(pipeline) = pipeline.as_mut()
            {
                pipeline.observe(event);
            }
        });
    }
    // SAFETY: forward unchanged; never suppress other applications' input.
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, message, data) }
}

unsafe extern "system" fn input_window(window: HWND, message: u32, wp: usize, lp: isize) -> isize {
    if message == WM_INPUT {
        let mut raw = RAWINPUT::default();
        let mut size = size_of::<RAWINPUT>() as u32;
        // SAFETY: the stack buffer is aligned and its capacity passed exactly.
        // Only keyboard TLC is registered; never interpret an unchecked union.
        let read = unsafe {
            GetRawInputData(
                lp as _,
                RID_INPUT,
                (&mut raw as *mut RAWINPUT).cast(),
                &mut size,
                size_of::<RAWINPUTHEADER>() as u32,
            )
        };
        PIPELINE.with(|pipeline| {
            if let Ok(mut pipeline) = pipeline.try_borrow_mut()
                && let Some(pipeline) = pipeline.as_mut()
            {
                if read == u32::MAX
                    || (read as usize) < size_of::<RAWINPUTHEADER>() + size_of::<RAWKEYBOARD>()
                {
                    saturating_add(&pipeline.runtime.windows.read_errors, 1);
                    pipeline.runtime.set_health(InputHealth::Degraded);
                } else if raw.header.dwType == RIM_TYPEKEYBOARD {
                    // SAFETY: buffer size and keyboard discriminator validated.
                    // Key identity is transient repeat-filter state only, never
                    // queued, serialized, logged or converted to characters.
                    let key = unsafe { raw.data.keyboard };
                    pipeline.observe(raw_keyboard_candidate(
                        key.Message,
                        key.VKey,
                        key.MakeCode,
                        key.Flags,
                    ));
                }
            }
        });
    } else if message == WM_INPUT_DEVICE_CHANGE {
        // A removed keyboard may never deliver key-up. Clear transient state.
        PIPELINE.with(|pipeline| {
            if let Some(pipeline) = pipeline.borrow_mut().as_mut() {
                pipeline.repeat_filter = WindowsRepeatFilter::new();
            }
        });
    }
    // SAFETY: required WM_INPUT cleanup and default message handling.
    unsafe { DefWindowProcW(window, message, wp, lp) }
}

struct Source {
    window: HWND,
    module: HINSTANCE,
    mouse: HHOOK,
    registered: bool,
}

const CLASS: windows_sys::core::PCWSTR = windows_sys::core::w!("CCFarmRawKeyboardW2");

impl Source {
    fn install() -> Result<Self, u32> {
        // SAFETY: all objects are created/used/destroyed on this input thread;
        // class name and callbacks have static lifetime.
        unsafe {
            let module = GetModuleHandleW(std::ptr::null());
            if module.is_null() {
                return Err(GetLastError());
            }
            let wc = WNDCLASSW {
                lpfnWndProc: Some(input_window),
                hInstance: module,
                lpszClassName: CLASS,
                ..Default::default()
            };
            if RegisterClassW(&wc) == 0 {
                return Err(GetLastError());
            }
            let mut source = Self {
                window: std::ptr::null_mut(),
                module,
                mouse: std::ptr::null_mut(),
                registered: false,
            };
            source.window = CreateWindowExW(
                0,
                CLASS,
                CLASS,
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            );
            if source.window.is_null() {
                return Err(GetLastError());
            }
            // Tauri's event loop already exists before app.setup starts us.
            // This application owns keyboard Raw Input from here until exit.
            // Do not use NOLEGACY or EXINPUTSINK: no suppression, and background
            // delivery must not depend on another app's device registration.
            let device = RAWINPUTDEVICE {
                usUsagePage: 1,
                usUsage: 6,
                dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
                hwndTarget: source.window,
            };
            if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
                return Err(GetLastError());
            }
            source.registered = true;
            source.mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0);
            if source.mouse.is_null() {
                return Err(GetLastError());
            }
            Ok(source)
        }
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        // SAFETY: each owned handle remains on its creation thread. Cleanup
        // also runs if installation fails halfway through.
        unsafe {
            if !self.mouse.is_null() {
                UnhookWindowsHookEx(self.mouse);
            }
            if self.registered {
                let device = RAWINPUTDEVICE {
                    usUsagePage: 1,
                    usUsage: 6,
                    dwFlags: RIDEV_REMOVE,
                    hwndTarget: std::ptr::null_mut(),
                };
                RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32);
            }
            if !self.window.is_null() {
                DestroyWindow(self.window);
            }
            UnregisterClassW(CLASS, self.module);
        }
    }
}

pub(super) fn spawn_event_tap(
    sender: SyncSender<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    stop: Arc<AtomicBool>,
    event_thread_id: Arc<AtomicU32>,
    on_fatal_unavailable: Arc<dyn Fn() + Send + Sync>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ccfarm-input-raw-w2".into())
        .spawn(move || {
            runtime.set_health(InputHealth::Starting);
            PIPELINE.with(|p| {
                *p.borrow_mut() = Some(WindowsInputPipeline::new(sender, Arc::clone(&runtime)))
            });
            let source = match Source::install() {
                Ok(source) => source,
                Err(_) => {
                    PIPELINE.with(|p| *p.borrow_mut() = None);
                    runtime.set_permission(PermissionState::Unavailable);
                    runtime.set_health(InputHealth::Stopped);
                    on_fatal_unavailable();
                    return;
                }
            };
            runtime.set_permission(PermissionState::Allowed);
            runtime.set_health(InputHealth::Healthy);
            // SAFETY: creating the message window already created this queue.
            event_thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            let mut message = MSG::default();
            while !stop.load(Ordering::Acquire) {
                // SAFETY: owned message queue and writable MSG; WM_QUIT wakes exit.
                let result = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
                if result <= 0 {
                    break;
                }
                // SAFETY: dispatch messages returned by GetMessage on their owner.
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
            event_thread_id.store(0, Ordering::Release);
            drop(source);
            PIPELINE.with(|p| *p.borrow_mut() = None);
            runtime.set_health(InputHealth::Stopped);
        })
        .expect("failed to spawn Windows input thread")
}
