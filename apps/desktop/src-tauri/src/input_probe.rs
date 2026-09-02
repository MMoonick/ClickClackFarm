use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 4096;
const TAP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PERMISSION_RECHECK_INTERVAL: Duration = Duration::from_secs(1);
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
}

impl RuntimeProbe {
    pub fn new() -> Self {
        Self {
            permission: AtomicU8::new(PermissionState::Unknown as u8),
            health: AtomicU8::new(InputHealth::Starting as u8),
            total_effective_inputs: AtomicU64::new(0),
            dropped_observations: AtomicU64::new(0),
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
        let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
        let aggregate_thread = spawn_aggregator(receiver, Arc::clone(&runtime), Arc::clone(&stop));
        let tap_thread = spawn_event_tap(
            sender,
            Arc::clone(&runtime),
            Arc::clone(&stop),
            on_fatal_unavailable,
        );

        *running = Some(RunningProbe {
            stop,
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

#[cfg(not(target_os = "macos"))]
fn spawn_event_tap(
    _sender: SyncSender<ObservedInput>,
    runtime: Arc<RuntimeProbe>,
    _stop: Arc<AtomicBool>,
    on_fatal_unavailable: Arc<dyn Fn() + Send + Sync>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        runtime.set_permission(PermissionState::Unavailable);
        runtime.set_health(InputHealth::Stopped);
        on_fatal_unavailable();
    })
}

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

#[cfg(not(target_os = "macos"))]
pub fn preflight_permission() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub fn request_permission() -> bool {
    // SAFETY: public CoreGraphics permission request; invocation follows an
    // explicit in-app explanation and does not inspect event content.
    unsafe { CGRequestListenEventAccess() }
}

#[cfg(not(target_os = "macos"))]
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
}
