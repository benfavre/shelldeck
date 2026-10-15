//! CEF's NSApplication contract lives only in the browser helper process.
use cef::application_mac::{CefAppProtocol, CrAppControlProtocol, CrAppProtocol};
use objc2::{
    define_class, extern_methods, msg_send,
    rc::Retained,
    runtime::{AnyObject, Bool},
    DefinedClass, MainThreadMarker,
};
use objc2_app_kit::{NSApp, NSApplication, NSApplicationActivationPolicy, NSEvent};
use std::cell::Cell;

#[derive(Default)]
pub struct BrowserApplicationIvars {
    handling_send_event: Cell<Bool>,
}

define_class!(
    #[unsafe(super(NSApplication))]
    #[ivars = BrowserApplicationIvars]
    pub struct BrowserApplication;

    impl BrowserApplication {
        #[unsafe(method(sendEvent:))]
        unsafe fn send_event(&self, event: &NSEvent) {
            let was_sending = self.ivars().handling_send_event.replace(Bool::YES);
            unsafe { let _: () = msg_send![super(self), sendEvent:event]; }
            self.ivars().handling_send_event.set(was_sending);
        }
        #[unsafe(method(terminate:))]
        unsafe fn terminate(&self, _sender: Option<&AnyObject>) { super::post(super::Work::Command(crate::protocol::Command::Shutdown)); }
    }
    unsafe impl CrAppControlProtocol for BrowserApplication {
        #[unsafe(method(setHandlingSendEvent:))]
        unsafe fn set_handling_send_event(&self, handling: Bool) { self.ivars().handling_send_event.set(handling); }
    }
    unsafe impl CrAppProtocol for BrowserApplication {
        #[unsafe(method(isHandlingSendEvent))]
        unsafe fn is_handling_send_event(&self) -> Bool { self.ivars().handling_send_event.get() }
    }
    unsafe impl CefAppProtocol for BrowserApplication {}
);
impl BrowserApplication {
    extern_methods! { #[unsafe(method(sharedApplication))] fn shared_application() -> Retained<Self>; }
}
pub(super) fn setup() {
    let _application = BrowserApplication::shared_application();
    let application = NSApp(MainThreadMarker::new().expect("CEF requires the main thread"));
    application.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
}
