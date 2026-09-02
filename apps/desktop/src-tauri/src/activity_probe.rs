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

#[cfg(not(target_os = "macos"))]
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
