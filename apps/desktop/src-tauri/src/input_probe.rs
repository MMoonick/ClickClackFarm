use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 4096;
const TAP_POLL_INTERVAL: Duration = Duration::from_millis(100);
#[cfg(target_os = "windows")]
#[path = "windows_input.rs"]
mod windows_input;
#[cfg(target_os = "macos")]
const PERMISSION_RECHECK_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(target_os = "macos")]
const MAX_REENABLE_ATTEMPTS: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Unknown = 0,
    Allowed = 1,
    Denied = 2,
    Unavailable = 3,
}

impl PermissionState {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Allowed,
            2 => Self::Denied,
            3 => Self::Unavailable,
            _ => Self::Unknown,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[repr(u8)]
#[serde(rename_all = "snake_case")]
pub enum InputHealth {
    Starting = 0,
    Healthy = 1,
    Degraded = 2,
    Stopped = 3,
}

impl InputHealth {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::Healthy,
            2 => Self::Degraded,
            3 => Self::Stopped,
            _ => Self::Starting,
        }
    }
}

pub struct RuntimeProbe {
    permission: AtomicU8,
    health: AtomicU8,
    total_effective_inputs: AtomicU64,
    dropped_observations: AtomicU64,
    #[cfg(any(target_os = "windows", test))]
    windows: WindowsInputCounters,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Default)]
struct WindowsInputCounters {
    key_down: AtomicU64,
    key_up: AtomicU64,
    keyboard_accepted: AtomicU64,
    mouse_accepted: AtomicU64,
    read_errors: AtomicU64,
}

impl RuntimeProbe {
    pub fn new() -> Self {
        Self {
            permission: AtomicU8::new(PermissionState::Unknown as u8),
            health: AtomicU8::new(InputHealth::Starting as u8),
            total_effective_inputs: AtomicU64::new(0),
            dropped_observations: AtomicU64::new(0),
            #[cfg(any(target_os = "windows", test))]
            windows: WindowsInputCounters::default(),
        }
    }

    pub fn permission(&self) -> PermissionState {
        PermissionState::from_raw(self.permission.load(Ordering::Acquire))
    }

    pub fn set_permission(&self, value: PermissionState) {
        self.permission.store(value as u8, Ordering::Release);
    }

    pub fn health(&self) -> InputHealth {
        InputHealth::from_raw(self.health.load(Ordering::Acquire))
    }

    pub fn set_health(&self, value: InputHealth) {
        self.health.store(value as u8, Ordering::Release);
    }

    pub fn total_effective_inputs(&self) -> u64 {
        self.total_effective_inputs.load(Ordering::Acquire)
    }

    fn add_effective_inputs(&self, count: u64) {
        saturating_add(&self.total_effective_inputs, count);
    }

    fn record_queue_drop(&self) {
        saturating_add(&self.dropped_observations, 1);
        self.set_health(InputHealth::Degraded);
    }
}

impl Default for RuntimeProbe {
    fn default() -> Self {
        Self::new()
    }
}

fn saturating_add(target: &AtomicU64, amount: u64) {
    let _ = target.fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
        Some(current.saturating_add(amount))
    });
}

struct ObservedInput {
    sequence: u64,
    observed_at: Instant,
}

#[cfg(target_os = "macos")]
enum CandidateEvent {
    KeyDown { autorepeat: bool },
    LeftMouseDown,
    RightMouseDown,
    OtherMouseDown { transient_button_number: i64 },
    Ignored,
}

#[cfg(any(target_os = "windows", test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WindowsCandidateEvent {
    KeyDown { virtual_key: u8 },
    KeyUp { virtual_key: u8 },
    LeftMouseDown,
    RightMouseDown,
    MiddleMouseDown,
    Ignored,
}

#[cfg(any(target_os = "windows", test))]
const WINDOWS_WM_KEYDOWN: u32 = 0x0100;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_WM_KEYUP: u32 = 0x0101;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_WM_SYSKEYDOWN: u32 = 0x0104;
#[cfg(any(target_os = "windows", test))]
const WINDOWS_WM_SYSKEYUP: u32 = 0x0105;

#[cfg(any(target_os = "windows", test))]
fn windows_keyboard_candidate(message: u32, virtual_key: u32) -> WindowsCandidateEvent {
    let Ok(virtual_key) = u8::try_from(virtual_key) else {
        return WindowsCandidateEvent::Ignored;
    };
    match message {
        WINDOWS_WM_KEYDOWN | WINDOWS_WM_SYSKEYDOWN => {
            WindowsCandidateEvent::KeyDown { virtual_key }
        }
        WINDOWS_WM_KEYUP | WINDOWS_WM_SYSKEYUP => WindowsCandidateEvent::KeyUp { virtual_key },
        _ => WindowsCandidateEvent::Ignored,
    }
}

// Raw Input uses generic modifier VKs. Distinguish left/right while keeping
// only a transient pressed-bit map; never decode or serialize input content.
#[cfg(any(target_os = "windows", test))]
fn raw_keyboard_candidate(
    message: u32,
    virtual_key: u16,
    scan: u16,
    flags: u16,
) -> WindowsCandidateEvent {
    if virtual_key == 0 || virtual_key >= 255 || scan == 255 {
        return WindowsCandidateEvent::Ignored;
    }
    let key = match virtual_key {
        0x10 => {
            if scan == 0x36 {
                0xa1
            } else {
                0xa0
            }
        }
        0x11 => {
            if flags & 2 != 0 {
                0xa3
            } else {
                0xa2
            }
        }
        0x12 => {
            if flags & 2 != 0 {
                0xa5
            } else {
                0xa4
            }
        }
        value => u32::from(value),
    };
    windows_keyboard_candidate(message, key)
}

#[cfg(any(target_os = "windows", test))]
struct WindowsInputPipeline {
    sender: SyncSender<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    next_sequence: u64,
    repeat_filter: WindowsRepeatFilter,
}

#[cfg(any(target_os = "windows", test))]
impl WindowsInputPipeline {
    fn new(sender: SyncSender<ObservedInput>, runtime: Arc<RuntimeProbe>) -> Self {
        Self {
            sender,
            runtime,
            next_sequence: 0,
            repeat_filter: WindowsRepeatFilter::new(),
        }
    }

    fn observe(&mut self, candidate: WindowsCandidateEvent) {
        match candidate {
            WindowsCandidateEvent::KeyDown { .. } => {
                saturating_add(&self.runtime.windows.key_down, 1)
            }
            WindowsCandidateEvent::KeyUp { .. } => saturating_add(&self.runtime.windows.key_up, 1),
            _ => {}
        }
        if !self.repeat_filter.observe(candidate) {
            return;
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        let observation = ObservedInput {
            sequence: self.next_sequence,
            observed_at: Instant::now(),
        };
        match self.sender.try_send(observation) {
            Ok(()) => {
                let counter = if matches!(candidate, WindowsCandidateEvent::KeyDown { .. }) {
                    &self.runtime.windows.keyboard_accepted
                } else {
                    &self.runtime.windows.mouse_accepted
                };
                saturating_add(counter, 1);
            }
            Err(TrySendError::Full(_)) => self.runtime.record_queue_drop(),
            Err(TrySendError::Disconnected(_)) => self.runtime.set_health(InputHealth::Stopped),
        }
    }
}

#[cfg(any(target_os = "windows", test))]
#[derive(Debug)]
struct WindowsRepeatFilter {
    pressed: [bool; 256],
}

#[cfg(any(target_os = "windows", test))]
impl WindowsRepeatFilter {
    fn new() -> Self {
        Self {
            pressed: [false; 256],
        }
    }

    fn observe(&mut self, candidate: WindowsCandidateEvent) -> bool {
        match candidate {
            WindowsCandidateEvent::KeyDown { virtual_key } => {
                let pressed = &mut self.pressed[usize::from(virtual_key)];
                if *pressed {
                    false
                } else {
                    *pressed = true;
                    true
                }
            }
            WindowsCandidateEvent::KeyUp { virtual_key } => {
                self.pressed[usize::from(virtual_key)] = false;
                false
            }
            WindowsCandidateEvent::LeftMouseDown
            | WindowsCandidateEvent::RightMouseDown
            | WindowsCandidateEvent::MiddleMouseDown => true,
            WindowsCandidateEvent::Ignored => false,
        }
    }
}

#[cfg(target_os = "macos")]
fn is_effective_candidate(candidate: CandidateEvent) -> bool {
    match candidate {
        CandidateEvent::KeyDown { autorepeat } => !autorepeat,
        CandidateEvent::LeftMouseDown | CandidateEvent::RightMouseDown => true,
        CandidateEvent::OtherMouseDown {
            transient_button_number,
        } => transient_button_number == 2,
        CandidateEvent::Ignored => false,
    }
}

struct RunningProbe {
    stop: Arc<AtomicBool>,
    event_thread_id: Arc<AtomicU32>,
    threads: Vec<JoinHandle<()>>,
}

pub struct InputController {
    running: Mutex<Option<RunningProbe>>,
}

impl InputController {
    pub fn new() -> Self {
        Self {
            running: Mutex::new(None),
        }
    }

    pub fn start(
        &self,
        runtime: Arc<RuntimeProbe>,
        on_fatal_unavailable: Arc<dyn Fn() + Send + Sync>,
    ) -> Result<(), String> {
        let mut running = self.running.lock().map_err(|_| "input lock poisoned")?;
        if running.is_some() {
            return Ok(());
        }

        let stop = Arc::new(AtomicBool::new(false));
        let event_thread_id = Arc::new(AtomicU32::new(0));
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        let aggregate_thread = spawn_aggregator(receiver, Arc::clone(&runtime), Arc::clone(&stop));
        let tap_thread = spawn_event_tap(
            sender,
            Arc::clone(&runtime),
            Arc::clone(&stop),
            Arc::clone(&event_thread_id),
            on_fatal_unavailable,
        );

        *running = Some(RunningProbe {
            stop,
            event_thread_id,
            threads: vec![aggregate_thread, tap_thread],
        });
        Ok(())
    }

    pub fn stop(&self, runtime: &RuntimeProbe) {
        let Ok(mut guard) = self.running.lock() else {
            runtime.set_health(InputHealth::Stopped);
            return;
        };
        let Some(running) = guard.take() else {
            runtime.set_health(InputHealth::Stopped);
            return;
        };
        running.stop.store(true, Ordering::Release);
        wake_event_thread(&running.event_thread_id);
        for thread in running.threads {
            let _ = thread.join();
        }
        runtime.set_health(InputHealth::Stopped);
    }
}

impl Default for InputController {
    fn default() -> Self {
        Self::new()
    }
}

fn spawn_aggregator(
    receiver: Receiver<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    stop: Arc<AtomicBool>,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("ccfarm-input-aggregate".into())
        .spawn(move || {
            let mut last_sequence = 0_u64;
            while !stop.load(Ordering::Acquire) {
                match receiver.recv_timeout(TAP_POLL_INTERVAL) {
                    Ok(observed) => {
                        if observed.sequence > last_sequence {
                            last_sequence = observed.sequence;
                            let _ = observed.observed_at;
                            runtime.add_effective_inputs(1);
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            }
            while let Ok(observed) = receiver.try_recv() {
                if observed.sequence > last_sequence {
                    last_sequence = observed.sequence;
                    let _ = observed.observed_at;
                    runtime.add_effective_inputs(1);
                }
            }
        })
        .expect("failed to spawn input aggregation thread")
}

#[cfg(target_os = "macos")]
fn spawn_event_tap(
    sender: SyncSender<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    stop: Arc<AtomicBool>,
    _event_thread_id: Arc<AtomicU32>,
    on_fatal_unavailable: Arc<dyn Fn() + Send + Sync>,
) -> JoinHandle<()> {
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult, EventField,
    };

    thread::Builder::new()
        .name("ccfarm-input-tap".into())
        .spawn(move || {
            runtime.set_health(InputHealth::Starting);
            let next_sequence = Arc::new(AtomicU64::new(0));
            let needs_reenable = Arc::new(AtomicBool::new(false));
            let callback_runtime = Arc::clone(&runtime);
            let callback_sequence = Arc::clone(&next_sequence);
            let callback_reenable = Arc::clone(&needs_reenable);

            let tap = CGEventTap::new(
                CGEventTapLocation::Session,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::ListenOnly,
                vec![
                    CGEventType::KeyDown,
                    CGEventType::LeftMouseDown,
                    CGEventType::RightMouseDown,
                    CGEventType::OtherMouseDown,
                ],
                move |_proxy, event_type, event| {
                    if matches!(
                        event_type,
                        CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
                    ) {
                        callback_runtime.set_health(InputHealth::Degraded);
                        callback_reenable.store(true, Ordering::Release);
                        return CallbackResult::Keep;
                    }

                    let candidate = match event_type {
                        CGEventType::KeyDown => CandidateEvent::KeyDown {
                            autorepeat: event
                                .get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT)
                                != 0,
                        },
                        CGEventType::LeftMouseDown => CandidateEvent::LeftMouseDown,
                        CGEventType::RightMouseDown => CandidateEvent::RightMouseDown,
                        CGEventType::OtherMouseDown => CandidateEvent::OtherMouseDown {
                            transient_button_number: event
                                .get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
                        },
                        _ => CandidateEvent::Ignored,
                    };

                    if is_effective_candidate(candidate) {
                        let sequence = callback_sequence
                            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                                Some(value.saturating_add(1))
                            })
                            .unwrap_or(u64::MAX)
                            .saturating_add(1);
                        let observation = ObservedInput {
                            sequence,
                            observed_at: Instant::now(),
                        };
                        match sender.try_send(observation) {
                            Ok(()) => {}
                            Err(TrySendError::Full(_)) => callback_runtime.record_queue_drop(),
                            Err(TrySendError::Disconnected(_)) => {
                                callback_runtime.set_health(InputHealth::Stopped)
                            }
                        }
                    }
                    CallbackResult::Keep
                },
            );

            let Ok(tap) = tap else {
                runtime.set_permission(PermissionState::Unavailable);
                runtime.set_health(InputHealth::Stopped);
                on_fatal_unavailable();
                return;
            };
            let Ok(source) = tap.mach_port().create_runloop_source(0) else {
                runtime.set_permission(PermissionState::Unavailable);
                runtime.set_health(InputHealth::Stopped);
                on_fatal_unavailable();
                return;
            };

            let run_loop = CFRunLoop::get_current();
            run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
            tap.enable();
            runtime.set_health(InputHealth::Healthy);

            let mut last_permission_check = Instant::now();
            let mut reenable_attempts = 0_u8;
            while !stop.load(Ordering::Acquire) {
                let _ = CFRunLoop::run_in_mode(
                    unsafe { kCFRunLoopDefaultMode },
                    TAP_POLL_INTERVAL,
                    false,
                );

                if last_permission_check.elapsed() >= PERMISSION_RECHECK_INTERVAL {
                    last_permission_check = Instant::now();
                    if !preflight_permission() {
                        runtime.set_permission(PermissionState::Denied);
                        runtime.set_health(InputHealth::Stopped);
                        on_fatal_unavailable();
                        break;
                    }
                }

                if needs_reenable.swap(false, Ordering::AcqRel) {
                    if reenable_attempts < MAX_REENABLE_ATTEMPTS {
                        reenable_attempts += 1;
                        thread::sleep(Duration::from_millis(100 * u64::from(reenable_attempts)));
                        tap.enable();
                        runtime.set_health(InputHealth::Healthy);
                    } else {
                        runtime.set_health(InputHealth::Stopped);
                        on_fatal_unavailable();
                        break;
                    }
                }
            }
            run_loop.remove_source(&source, unsafe { kCFRunLoopCommonModes });
            runtime.set_health(InputHealth::Stopped);
        })
        .expect("failed to spawn event tap thread")
}

#[cfg(target_os = "windows")]
use windows_input::spawn_event_tap;

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn spawn_event_tap(
    _sender: SyncSender<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    _stop: Arc<AtomicBool>,
    _event_thread_id: Arc<AtomicU32>,
    on_fatal_unavailable: Arc<dyn Fn() + Send + Sync>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        runtime.set_permission(PermissionState::Unavailable);
        runtime.set_health(InputHealth::Stopped);
        on_fatal_unavailable();
    })
}

#[cfg(target_os = "windows")]
fn wake_event_thread(event_thread_id: &AtomicU32) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};

    let thread_id = event_thread_id.load(Ordering::Acquire);
    if thread_id != 0 {
        // SAFETY: the hook thread publishes its ID only after creating its
        // message queue. WM_QUIT is used solely to end that owned loop.
        let _ = unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
    }
}

#[cfg(not(target_os = "windows"))]
fn wake_event_thread(_event_thread_id: &AtomicU32) {}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
}

#[cfg(target_os = "macos")]
pub fn preflight_permission() -> bool {
    // SAFETY: public CoreGraphics permission query; it takes no pointers and
    // does not expose event content.
    unsafe { CGPreflightListenEventAccess() }
}

#[cfg(target_os = "windows")]
pub fn preflight_permission() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn preflight_permission() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn request_permission() -> bool {
    // SAFETY: public CoreGraphics permission request; invocation follows an
    // explicit in-app explanation and does not inspect event content.
    unsafe { CGRequestListenEventAccess() }
}

#[cfg(target_os = "windows")]
pub fn request_permission() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn request_permission() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_counter_saturates() {
        let counter = AtomicU64::new(u64::MAX - 1);
        saturating_add(&counter, 8);
        assert_eq!(counter.load(Ordering::Acquire), u64::MAX);
    }

    #[test]
    fn raw_keyboard_to_aggregator_counts_ten_taps_and_three_clicks_once() {
        let runtime = Arc::new(RuntimeProbe::new());
        let (sender, receiver) = sync_channel(64);
        let worker = spawn_aggregator(
            receiver,
            Arc::clone(&runtime),
            Arc::new(AtomicBool::new(false)),
        );
        let mut pipeline = WindowsInputPipeline::new(sender, Arc::clone(&runtime));
        for _ in 0..10 {
            pipeline.observe(raw_keyboard_candidate(WINDOWS_WM_KEYDOWN, 65, 30, 0));
            // Holding a key must not generate more gameplay inputs.
            pipeline.observe(raw_keyboard_candidate(WINDOWS_WM_KEYDOWN, 65, 30, 0));
            pipeline.observe(raw_keyboard_candidate(WINDOWS_WM_KEYUP, 65, 30, 1));
        }
        pipeline.observe(WindowsCandidateEvent::LeftMouseDown);
        pipeline.observe(WindowsCandidateEvent::RightMouseDown);
        pipeline.observe(WindowsCandidateEvent::MiddleMouseDown);
        drop(pipeline); // channel closes; aggregator drains before joining.
        worker.join().unwrap();
        assert_eq!(runtime.windows.key_down.load(Ordering::Acquire), 20);
        assert_eq!(runtime.windows.key_up.load(Ordering::Acquire), 10);
        assert_eq!(
            runtime.windows.keyboard_accepted.load(Ordering::Acquire),
            10
        );
        assert_eq!(runtime.windows.mouse_accepted.load(Ordering::Acquire), 3);
        assert_eq!(runtime.windows.read_errors.load(Ordering::Acquire), 0);
        assert_eq!(runtime.total_effective_inputs(), 13);
    }

    #[test]
    fn raw_input_distinguishes_modifiers_and_ignores_invalid_keys() {
        let mut filter = WindowsRepeatFilter::new();
        for (vk, left_scan, right_scan, right_flags) in [
            (16, 0x2a, 0x36, 0),
            (17, 0x1d, 0x1d, 2),
            (18, 0x38, 0x38, 2),
        ] {
            assert!(filter.observe(raw_keyboard_candidate(WINDOWS_WM_KEYDOWN, vk, left_scan, 0)));
            assert!(filter.observe(raw_keyboard_candidate(
                WINDOWS_WM_KEYDOWN,
                vk,
                right_scan,
                right_flags
            )));
            assert!(!filter.observe(raw_keyboard_candidate(
                WINDOWS_WM_KEYDOWN,
                vk,
                right_scan,
                right_flags
            )));
        }
        for (vk, scan) in [(0, 0), (255, 1), (256, 1), (65, 255)] {
            assert_eq!(
                raw_keyboard_candidate(WINDOWS_WM_KEYDOWN, vk, scan, 0),
                WindowsCandidateEvent::Ignored
            );
        }
    }

    #[test]
    fn windows_pipeline_reports_queue_overflow_without_blocking() {
        let runtime = Arc::new(RuntimeProbe::new());
        let (sender, _receiver) = sync_channel(1);
        let mut pipeline = WindowsInputPipeline::new(sender, Arc::clone(&runtime));
        pipeline.observe(WindowsCandidateEvent::LeftMouseDown);
        pipeline.observe(raw_keyboard_candidate(WINDOWS_WM_KEYDOWN, 65, 30, 0));
        assert_eq!(runtime.dropped_observations.load(Ordering::Acquire), 1);
        assert_eq!(runtime.windows.keyboard_accepted.load(Ordering::Acquire), 0);
        assert_eq!(runtime.health(), InputHealth::Degraded);
    }

    #[test]
    fn public_state_encodings_are_stable() {
        assert_eq!(PermissionState::from_raw(1), PermissionState::Allowed);
        assert_eq!(PermissionState::from_raw(255), PermissionState::Unknown);
        assert_eq!(InputHealth::from_raw(2), InputHealth::Degraded);
        assert_eq!(InputHealth::from_raw(255), InputHealth::Starting);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn filters_repeat_and_extended_mouse_buttons() {
        assert!(is_effective_candidate(CandidateEvent::KeyDown {
            autorepeat: false,
        }));
        assert!(!is_effective_candidate(CandidateEvent::KeyDown {
            autorepeat: true,
        }));
        assert!(is_effective_candidate(CandidateEvent::LeftMouseDown));
        assert!(is_effective_candidate(CandidateEvent::RightMouseDown));
        assert!(is_effective_candidate(CandidateEvent::OtherMouseDown {
            transient_button_number: 2,
        }));
        assert!(!is_effective_candidate(CandidateEvent::OtherMouseDown {
            transient_button_number: 3,
        }));
        assert!(!is_effective_candidate(CandidateEvent::Ignored));
    }

    #[test]
    fn windows_filter_counts_one_keydown_until_keyup_and_three_mouse_buttons() {
        let mut filter = WindowsRepeatFilter::new();
        assert!(filter.observe(WindowsCandidateEvent::KeyDown { virtual_key: 65 }));
        assert!(!filter.observe(WindowsCandidateEvent::KeyDown { virtual_key: 65 }));
        assert!(!filter.observe(WindowsCandidateEvent::KeyUp { virtual_key: 65 }));
        assert!(filter.observe(WindowsCandidateEvent::KeyDown { virtual_key: 65 }));
        assert!(filter.observe(WindowsCandidateEvent::LeftMouseDown));
        assert!(filter.observe(WindowsCandidateEvent::RightMouseDown));
        assert!(filter.observe(WindowsCandidateEvent::MiddleMouseDown));
        assert!(!filter.observe(WindowsCandidateEvent::Ignored));
    }

    #[test]
    fn windows_keyboard_messages_map_to_transient_key_state_without_character_data() {
        assert_eq!(
            windows_keyboard_candidate(WINDOWS_WM_KEYDOWN, 65),
            WindowsCandidateEvent::KeyDown { virtual_key: 65 }
        );
        assert_eq!(
            windows_keyboard_candidate(WINDOWS_WM_SYSKEYDOWN, 18),
            WindowsCandidateEvent::KeyDown { virtual_key: 18 }
        );
        assert_eq!(
            windows_keyboard_candidate(WINDOWS_WM_KEYUP, 65),
            WindowsCandidateEvent::KeyUp { virtual_key: 65 }
        );
        assert_eq!(
            windows_keyboard_candidate(WINDOWS_WM_SYSKEYUP, 18),
            WindowsCandidateEvent::KeyUp { virtual_key: 18 }
        );
        assert_eq!(
            windows_keyboard_candidate(0x0201, 65),
            WindowsCandidateEvent::Ignored
        );
        assert_eq!(
            windows_keyboard_candidate(WINDOWS_WM_KEYDOWN, 256),
            WindowsCandidateEvent::Ignored
        );
    }
}
