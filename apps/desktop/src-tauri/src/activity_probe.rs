use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const MAX_UNOBSERVED_GAP: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, Default)]
struct ActivityGate {
    session_active: bool,
    system_awake: bool,
    screens_awake: bool,
    any_display_awake: bool,
}

impl ActivityGate {
    fn productive(self) -> bool {
        self.session_active && self.system_awake && self.screens_awake && self.any_display_awake
    }
}

struct Timeline {
    gate: ActivityGate,
    anchor: Instant,
    pending_productive: Duration,
}

impl Timeline {
    fn new(now: Instant) -> Self {
        Self {
            gate: ActivityGate::default(),
            anchor: now,
            pending_productive: Duration::ZERO,
        }
    }

    fn settle_to(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.anchor);
        if self.gate.productive() && elapsed <= MAX_UNOBSERVED_GAP {
            self.pending_productive = self.pending_productive.saturating_add(elapsed);
        }
        self.anchor = now;
    }

    fn update(&mut self, now: Instant, update: impl FnOnce(&mut ActivityGate)) {
        self.settle_to(now);
        update(&mut self.gate);
    }

    fn take_sample(&mut self, now: Instant) -> Duration {
        self.settle_to(now);
        std::mem::take(&mut self.pending_productive)
    }
}

pub struct ActivityProbe {
    timeline: Mutex<Timeline>,
}

impl ActivityProbe {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            timeline: Mutex::new(Timeline::new(Instant::now())),
        })
    }

    pub fn install(self: &Arc<Self>) -> Result<(), String> {
        platform::install_notifications(self)?;
        let display_awake = platform::any_online_display_awake().unwrap_or(false);
        self.update(|gate| {
            // Setup is running in a live user process. Notifications take over
            // these flags immediately after registration; display state is
            // reconciled from Core Graphics before the gate can become true.
            gate.session_active = true;
            gate.system_awake = true;
            gate.screens_awake = display_awake;
            gate.any_display_awake = display_awake;
        });
        Ok(())
    }

    pub fn sample(&self) -> Duration {
        let display_awake = platform::any_online_display_awake().unwrap_or(false);
        let now = Instant::now();
        let Ok(mut timeline) = self.timeline.lock() else {
            return Duration::ZERO;
        };
        timeline.update(now, |gate| gate.any_display_awake = display_awake);
        timeline.take_sample(now)
    }

    fn update(&self, update: impl FnOnce(&mut ActivityGate)) {
        if let Ok(mut timeline) = self.timeline.lock() {
            timeline.update(Instant::now(), update);
        }
    }

    fn set_session_active(&self, active: bool) {
        self.update(|gate| gate.session_active = active);
    }

    fn set_system_awake(&self, awake: bool) {
        self.update(|gate| gate.system_awake = awake);
    }

    fn set_screens_awake(&self, awake: bool) {
        self.update(|gate| gate.screens_awake = awake);
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::ActivityProbe;
    use block2::RcBlock;
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidSleepNotification,
        NSWorkspaceScreensDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};
    use std::{ptr, ptr::NonNull, sync::Arc};

    type DisplayId = u32;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGGetOnlineDisplayList(
            max_displays: u32,
            online_displays: *mut DisplayId,
            display_count: *mut u32,
        ) -> i32;
        fn CGDisplayIsAsleep(display: DisplayId) -> u32;
    }

    pub fn any_online_display_awake() -> Result<bool, String> {
        let mut count = 0_u32;
        // SAFETY: Core Graphics accepts a null buffer when max_displays is 0.
        if unsafe { CGGetOnlineDisplayList(0, ptr::null_mut(), &mut count) } != 0 || count == 0 {
            return Err("online display query failed".to_owned());
        }
        let mut displays = vec![0_u32; count as usize];
        let mut actual = 0_u32;
        // SAFETY: displays owns space for count IDs and actual is writable.
        if unsafe { CGGetOnlineDisplayList(count, displays.as_mut_ptr(), &mut actual) } != 0 {
            return Err("online display query failed".to_owned());
        }
        displays.truncate(actual as usize);
        Ok(displays
            .into_iter()
            // SAFETY: IDs came directly from CGGetOnlineDisplayList.
            .any(|display| unsafe { CGDisplayIsAsleep(display) == 0 }))
    }

    pub fn install_notifications(probe: &Arc<ActivityProbe>) -> Result<(), String> {
        let workspace = NSWorkspace::sharedWorkspace();
        let center = workspace.notificationCenter();

        register(
            &center,
            unsafe { NSWorkspaceSessionDidBecomeActiveNotification },
            probe,
            |probe| probe.set_session_active(true),
        );
        register(
            &center,
            unsafe { NSWorkspaceSessionDidResignActiveNotification },
            probe,
            |probe| probe.set_session_active(false),
        );
        register(
            &center,
            unsafe { NSWorkspaceWillSleepNotification },
            probe,
            |probe| probe.set_system_awake(false),
        );
        register(
            &center,
            unsafe { NSWorkspaceDidWakeNotification },
            probe,
            |probe| probe.set_system_awake(true),
        );
        register(
            &center,
            unsafe { NSWorkspaceScreensDidSleepNotification },
            probe,
            |probe| probe.set_screens_awake(false),
        );
        register(
            &center,
            unsafe { NSWorkspaceScreensDidWakeNotification },
            probe,
            |probe| probe.set_screens_awake(true),
        );
        Ok(())
    }

    fn register(
        center: &NSNotificationCenter,
        name: &NSNotificationName,
        probe: &Arc<ActivityProbe>,
        callback: fn(&ActivityProbe),
    ) {
        let probe = Arc::clone(probe);
        let block = RcBlock::new(move |_notification: NonNull<NSNotification>| callback(&probe));
        // SAFETY: the notification name and callback signature are supplied by
        // AppKit. The center retains the opaque observer for process lifetime.
        let observer = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(observer);
    }
}

#[cfg(target_os = "windows")]
mod platform {
    use super::ActivityProbe;
    use std::{
        cell::RefCell,
        mem::size_of,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::sync_channel,
        },
        thread,
    };
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        System::{
            LibraryLoader::GetModuleHandleW,
            Power::{
                POWERBROADCAST_SETTING, RegisterPowerSettingNotification,
                UnregisterPowerSettingNotification,
            },
            RemoteDesktop::{
                NOTIFY_FOR_THIS_SESSION, WTSRegisterSessionNotification,
                WTSUnRegisterSessionNotification,
            },
            SystemServices::GUID_SESSION_DISPLAY_STATUS,
        },
        UI::WindowsAndMessaging::{
            CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow,
            DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC,
            PBT_APMRESUMECRITICAL, PBT_APMRESUMESUSPEND, PBT_APMSUSPEND, PBT_POWERSETTINGCHANGE,
            RegisterClassW, TranslateMessage, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE, WNDCLASSW,
            WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
        },
    };

    static DISPLAY_AWAKE: AtomicBool = AtomicBool::new(true);

    thread_local! {
        static ACTIVITY_PROBE: RefCell<Option<Arc<ActivityProbe>>> = const { RefCell::new(None) };
    }

    pub fn any_online_display_awake() -> Result<bool, String> {
        Ok(DISPLAY_AWAKE.load(Ordering::Acquire))
    }

    pub fn install_notifications(probe: &Arc<ActivityProbe>) -> Result<(), String> {
        let (ready_sender, ready_receiver) = sync_channel(1);
        let probe = Arc::clone(probe);
        thread::Builder::new()
            .name("ccfarm-activity-events".into())
            .spawn(move || run_notification_window(probe, ready_sender))
            .map_err(|error| format!("failed to start Windows activity probe: {error}"))?;
        ready_receiver
            .recv()
            .map_err(|_| "Windows activity probe stopped during setup".to_owned())?
    }

    fn run_notification_window(
        probe: Arc<ActivityProbe>,
        ready: std::sync::mpsc::SyncSender<Result<(), String>>,
    ) {
        ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = Some(Arc::clone(&probe)));
        let class_name: Vec<u16> = "ClickClackFarmActivityProbe\0".encode_utf16().collect();
        // SAFETY: a null module name requests the current executable module.
        let module = unsafe { GetModuleHandleW(std::ptr::null()) };
        if module.is_null() {
            let _ = ready.send(Err(format!(
                "failed to resolve Windows application module: {}",
                std::io::Error::last_os_error()
            )));
            ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
            return;
        }
        let window_class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: class_name.as_ptr(),
            ..Default::default()
        };
        // SAFETY: window_class and its UTF-16 class name remain live for this
        // thread's message-loop lifetime.
        if unsafe { RegisterClassW(&window_class) } == 0 {
            let _ = ready.send(Err(format!(
                "failed to register Windows activity window: {}",
                std::io::Error::last_os_error()
            )));
            ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
            return;
        }
        // SAFETY: class_name names the class registered above. A message-only
        // window has no visible surface or user interaction.
        let window = unsafe {
            CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                std::ptr::null_mut(),
                module,
                std::ptr::null(),
            )
        };
        if window.is_null() {
            let _ = ready.send(Err(format!(
                "failed to create Windows activity window: {}",
                std::io::Error::last_os_error()
            )));
            ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
            return;
        }
        // SAFETY: window is live and belongs to the current interactive session.
        if unsafe { WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION) } == 0 {
            let _ = ready.send(Err(format!(
                "failed to register Windows session notifications: {}",
                std::io::Error::last_os_error()
            )));
            // SAFETY: window was created above and is owned by this thread.
            let _ = unsafe { DestroyWindow(window) };
            ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
            return;
        }
        // SAFETY: window is a valid HWND recipient and the GUID is static.
        let display_registration = unsafe {
            RegisterPowerSettingNotification(
                window.cast(),
                &GUID_SESSION_DISPLAY_STATUS,
                DEVICE_NOTIFY_WINDOW_HANDLE,
            )
        };
        if display_registration == 0 {
            let _ = ready.send(Err(format!(
                "failed to register Windows display notifications: {}",
                std::io::Error::last_os_error()
            )));
            // SAFETY: the registration and window are both owned here.
            unsafe {
                let _ = WTSUnRegisterSessionNotification(window);
                let _ = DestroyWindow(window);
            }
            ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
            return;
        }

        let _ = ready.send(Ok(()));
        let mut message = MSG::default();
        // SAFETY: message is writable; this thread owns the notification window.
        while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
            // SAFETY: message came from GetMessageW on this thread.
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        // SAFETY: handles remain valid until the message loop ends.
        unsafe {
            let _ = UnregisterPowerSettingNotification(display_registration);
            let _ = WTSUnRegisterSessionNotification(window);
            let _ = DestroyWindow(window);
        }
        probe.set_session_active(false);
        probe.set_system_awake(false);
        probe.set_screens_awake(false);
        DISPLAY_AWAKE.store(false, Ordering::Release);
        ACTIVITY_PROBE.with(|slot| *slot.borrow_mut() = None);
    }

    unsafe extern "system" fn window_proc(
        window: HWND,
        message: u32,
        parameter: WPARAM,
        data: LPARAM,
    ) -> LRESULT {
        ACTIVITY_PROBE.with(|slot| {
            let Ok(slot) = slot.try_borrow() else {
                return;
            };
            let Some(probe) = slot.as_ref() else {
                return;
            };
            match message {
                WM_WTSSESSION_CHANGE => match parameter as u32 {
                    WTS_SESSION_LOCK => probe.set_session_active(false),
                    WTS_SESSION_UNLOCK => probe.set_session_active(true),
                    _ => {}
                },
                WM_POWERBROADCAST => match parameter as u32 {
                    PBT_APMSUSPEND => {
                        probe.set_system_awake(false);
                        probe.set_screens_awake(false);
                        DISPLAY_AWAKE.store(false, Ordering::Release);
                    }
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMECRITICAL | PBT_APMRESUMESUSPEND => {
                        probe.set_system_awake(true)
                    }
                    PBT_POWERSETTINGCHANGE if data != 0 => {
                        // SAFETY: PBT_POWERSETTINGCHANGE supplies a
                        // POWERBROADCAST_SETTING for the duration of the call.
                        let setting = unsafe { &*(data as *const POWERBROADCAST_SETTING) };
                        if same_guid(&setting.PowerSetting, &GUID_SESSION_DISPLAY_STATUS)
                            && setting.DataLength as usize >= size_of::<u32>()
                        {
                            // SAFETY: DataLength guarantees four readable bytes;
                            // notification payload alignment is not assumed.
                            let state = unsafe {
                                std::ptr::read_unaligned(setting.Data.as_ptr().cast::<u32>())
                            };
                            let awake = state != 0;
                            DISPLAY_AWAKE.store(awake, Ordering::Release);
                            probe.set_screens_awake(awake);
                        }
                    }
                    _ => {}
                },
                _ => {}
            }
        });
        // SAFETY: unhandled messages are delegated to the default window proc.
        unsafe { DefWindowProcW(window, message, parameter, data) }
    }

    fn same_guid(left: &windows_sys::core::GUID, right: &windows_sys::core::GUID) -> bool {
        left.data1 == right.data1
            && left.data2 == right.data2
            && left.data3 == right.data3
            && left.data4 == right.data4
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod platform {
    use super::ActivityProbe;
    use std::sync::Arc;

    pub fn any_online_display_awake() -> Result<bool, String> {
        Err("activity probe unavailable".to_owned())
    }

    pub fn install_notifications(_probe: &Arc<ActivityProbe>) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{ActivityGate, Timeline};
    use std::time::{Duration, Instant};

    #[test]
    fn inactive_time_never_becomes_productive_and_wake_does_not_backfill() {
        let start = Instant::now();
        let mut timeline = Timeline::new(start);
        timeline.update(start, |gate| {
            *gate = ActivityGate {
                session_active: true,
                system_awake: true,
                screens_awake: true,
                any_display_awake: true,
            };
        });
        assert_eq!(
            timeline.take_sample(start + Duration::from_secs(2)),
            Duration::from_secs(2)
        );

        timeline.update(start + Duration::from_secs(3), |gate| {
            gate.system_awake = false
        });
        timeline.update(start + Duration::from_secs(30), |gate| {
            gate.system_awake = true
        });
        assert_eq!(
            timeline.take_sample(start + Duration::from_secs(31)),
            Duration::from_secs(2)
        );
    }

    #[test]
    fn an_unobserved_long_gap_is_fail_closed() {
        let start = Instant::now();
        let mut timeline = Timeline::new(start);
        timeline.update(start, |gate| {
            *gate = ActivityGate {
                session_active: true,
                system_awake: true,
                screens_awake: true,
                any_display_awake: true,
            };
        });
        assert_eq!(
            timeline.take_sample(start + Duration::from_secs(6)),
            Duration::ZERO
        );
    }
}
