use crate::{AppEvent, web};
use std::{
    ffi::{CStr, CString, c_char},
    sync::OnceLock,
};
use tao::event_loop::EventLoopProxy;
static EVENTS: OnceLock<EventLoopProxy<AppEvent>> = OnceLock::new();
unsafe extern "C" {
    fn cockpit_dashboard(json: *const c_char, show: bool, callback: extern "C" fn(*const c_char));
    fn cockpit_style_menu(
        menu: *mut std::ffi::c_void,
        theme: *const c_char,
        color: *const c_char,
        direct: bool,
        this_mac: *const c_char,
        this_direct: bool,
    );
    fn cockpit_capture(path: *const c_char);
}
extern "C" fn action(json: *const c_char) {
    if json.is_null() {
        return;
    }
    let Ok(text) = (unsafe { CStr::from_ptr(json) }).to_str() else {
        return;
    };
    let Some(proxy) = EVENTS.get() else { return };
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text)
        && v["kind"] == "preferences"
    {
        let _ = proxy.send_event(AppEvent::NativePreferences(v["value"].clone()));
    } else if let Ok(request) = web::decode(text) {
        let _ = proxy.send_event(AppEvent::Web(request));
    }
}
pub fn init(proxy: EventLoopProxy<AppEvent>) {
    let _ = EVENTS.set(proxy);
}
pub fn update(state: &serde_json::Value, show: bool) {
    let json = CString::new(state.to_string()).expect("JSON has no literal NUL");
    unsafe { cockpit_dashboard(json.as_ptr(), show, action) };
}
pub fn capture(path: &std::path::Path) {
    let path = CString::new(path.to_string_lossy().as_bytes()).unwrap();
    unsafe { cockpit_capture(path.as_ptr()) };
}

pub fn style_menu(
    menu: *mut std::ffi::c_void,
    theme: &str,
    color: &str,
    direct: bool,
    this_mac: Option<&str>,
    this_direct: bool,
) {
    let this_mac = CString::new(this_mac.unwrap_or("")).unwrap();
    let theme = CString::new(theme).unwrap();
    let color = CString::new(color).unwrap();
    unsafe {
        cockpit_style_menu(
            menu,
            theme.as_ptr(),
            color.as_ptr(),
            direct,
            this_mac.as_ptr(),
            this_direct,
        );
    }
}
