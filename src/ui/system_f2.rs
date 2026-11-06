//! Optional foreground-only handling of F2 translated to brightness-increase.
//! Never changes HID properties, keyboard preferences, or other applications.
use crate::core::keyboard::{SystemF2Decision, SystemF2Phase, SystemF2Press, system_f2_phase};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType};
use objc2_core_foundation::{
    CFMachPort, CFRetained, CFRunLoop, CFRunLoopSource, kCFRunLoopCommonModes,
};
use objc2_core_graphics::{
    CGEvent, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventTapProxy,
    CGEventType,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    ptr::NonNull,
    time::{Duration, Instant},
};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    Off,
    NeedsPermission,
    Ready,
    Unavailable,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "꺼짐",
            Self::NeedsPermission => "손쉬운 사용 권한 필요",
            Self::Ready => "사용 가능 · 이 앱의 파일 목록에서만 동작",
            Self::Unavailable => "키 연결을 열 수 없음 · 권한 확인 후 앱을 다시 실행하세요",
        }
    }
}

#[derive(Default)]
pub struct Service {
    tap: Option<Tap>,
    status: Status,
    checked: Option<Instant>,
}
impl Service {
    pub fn status(&self) -> Status {
        if self
            .tap
            .as_ref()
            .is_some_and(|t| t.wanted_active.get() && !t.context.active.get())
        {
            Status::Unavailable
        } else {
            self.status
        }
    }
    pub fn sync(&mut self, enabled: bool, active: bool, force: bool) {
        if !enabled {
            self.tap = None;
            self.status = Status::Off;
            self.checked = None;
            return;
        }
        if let Some(tap) = &self.tap {
            tap.set_active(active);
        }
        if !force
            && self
                .checked
                .is_some_and(|t| t.elapsed() < Duration::from_secs(1))
        {
            return;
        }
        self.checked = Some(Instant::now());
        // This only checks consent. It does not prompt or grant permission.
        if !unsafe { AXIsProcessTrusted() } {
            self.tap = None;
            self.status = Status::NeedsPermission;
            return;
        }
        if force
            && self
                .tap
                .as_ref()
                .is_some_and(|t| active && !t.context.active.get())
        {
            self.tap = None;
        }
        if self.tap.is_none() && (force || self.status != Status::Unavailable) {
            self.tap = Tap::new();
        }
        self.status = if let Some(tap) = &self.tap {
            tap.set_active(active);
            Status::Ready
        } else {
            Status::Unavailable
        };
    }
    pub fn stop(&mut self) {
        self.tap = None;
    }
}

#[derive(Default)]
struct CallbackState {
    active: Cell<bool>,
    press: RefCell<SystemF2Press>,
}
struct Tap {
    port: CFRetained<CFMachPort>,
    source: CFRetained<CFRunLoopSource>,
    run_loop: CFRetained<CFRunLoop>,
    context: Box<CallbackState>,
    wanted_active: Cell<bool>,
}
impl Tap {
    fn new() -> Option<Self> {
        let mut context = Box::<CallbackState>::default();
        let port = unsafe {
            CGEvent::tap_create(
                CGEventTapLocation::SessionEventTap,
                CGEventTapPlacement::HeadInsertEventTap,
                CGEventTapOptions::Default,
                // NX_SYSDEFINED only: no letters, passwords, mouse, or ordinary key events.
                1 << 14,
                Some(callback),
                (&mut *context as *mut CallbackState).cast(),
            )?
        };
        CGEvent::tap_enable(&port, false);
        let source = CFMachPort::new_run_loop_source(None, Some(&port), 0)?;
        let run_loop = CFRunLoop::main()?;
        unsafe {
            run_loop.add_source(Some(&source), kCFRunLoopCommonModes);
        }
        Some(Self {
            port,
            source,
            run_loop,
            context,
            wanted_active: Cell::new(false),
        })
    }
    fn set_active(&self, active: bool) {
        if self.wanted_active.replace(active) != active {
            self.context.active.set(active);
            self.context.press.borrow_mut().reset();
            CGEvent::tap_enable(&self.port, active);
        }
    }
}
impl Drop for Tap {
    fn drop(&mut self) {
        self.context.active.set(false);
        CGEvent::tap_enable(&self.port, false);
        unsafe {
            self.run_loop
                .remove_source(Some(&self.source), kCFRunLoopCommonModes);
        }
        self.source.invalidate();
        self.port.invalidate();
    }
}

unsafe extern "C-unwind" fn callback(
    _: CGEventTapProxy,
    kind: CGEventType,
    event: NonNull<CGEvent>,
    info: *mut c_void,
) -> *mut CGEvent {
    let Some(mtm) = MainThreadMarker::new() else {
        return event.as_ptr();
    };
    let state = unsafe { &*info.cast::<CallbackState>() };
    if kind == CGEventType::TapDisabledByTimeout || kind == CGEventType::TapDisabledByUserInput {
        // Leave disabled until a later foreground activation/consent check.
        state.active.set(false);
        state.press.borrow_mut().reset();
        return event.as_ptr();
    }
    if kind.0 != 14 || !state.active.get() {
        return event.as_ptr();
    }
    // Do not rely on a possibly queued application-deactivation notification.
    if !super::is_system_rename_app_frontmost(mtm) {
        state.press.borrow_mut().reset();
        return event.as_ptr();
    }
    let Some(ns_event) = NSEvent::eventWithCGEvent(unsafe { event.as_ref() }) else {
        return event.as_ptr();
    };
    if ns_event.r#type() != NSEventType::SystemDefined {
        return event.as_ptr();
    }
    let Some(phase) = system_f2_phase(ns_event.subtype().0, unsafe { ns_event.data1() }) else {
        return event.as_ptr();
    };
    let modified = ns_event.modifierFlags().intersects(
        NSEventModifierFlags::Command
            | NSEventModifierFlags::Control
            | NSEventModifierFlags::Option
            | NSEventModifierFlags::Shift,
    );
    let target = if !modified && matches!(phase, SystemF2Phase::Down { repeat: false }) {
        super::system_rename_target(mtm)
    } else {
        None
    };
    let decision = state.press.borrow_mut().handle(phase, target.is_some());
    match decision {
        SystemF2Decision::Pass => event.as_ptr(),
        SystemF2Decision::Suppress => std::ptr::null_mut(),
        SystemF2Decision::Rename => {
            if let Some(target) = target {
                super::record_system_rename_input(&ns_event);
                // Keep the event tap fast and do not change AppKit focus from its callback.
                dispatch2::DispatchQueue::main()
                    .exec_async(move || super::perform_system_rename(target));
            }
            std::ptr::null_mut()
        }
    }
}
