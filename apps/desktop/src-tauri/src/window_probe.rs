use tauri::{AppHandle, Manager, WebviewWindow};

pub const MAIN_WINDOW_LABEL: &str = "main";

pub fn show_main_window(app: &AppHandle) -> Result<(), String> {
    let window = main_window(app)?;
    window.show().map_err(|_| "failed to show main window")?;
    window
        .set_focus()
        .map_err(|_| "failed to focus main window".to_owned())
}

pub fn hide_main_window(app: &AppHandle) -> Result<(), String> {
    main_window(app)?
        .hide()
        .map_err(|_| "failed to hide main window".to_owned())
}

pub fn configure_desktop_window(app: &AppHandle) -> Result<(), String> {
    set_main_window_always_on_top(app, true)?;
    #[cfg(target_os = "macos")]
    configure_collection_behavior(&main_window(app)?)?;
    Ok(())
}

pub fn set_main_window_always_on_top(app: &AppHandle, enabled: bool) -> Result<(), String> {
    main_window(app)?
        .set_always_on_top(enabled)
        .map_err(|_| "failed to update public floating level".to_owned())
}

fn main_window(app: &AppHandle) -> Result<WebviewWindow, String> {
    app.get_webview_window(MAIN_WINDOW_LABEL)
        .ok_or_else(|| "main window is unavailable".to_owned())
}

#[cfg(target_os = "macos")]
fn configure_collection_behavior(window: &WebviewWindow) -> Result<(), String> {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let raw = window
        .ns_window()
        .map_err(|_| "failed to access native main window")?;
    if raw.is_null() {
        return Err("native main window was null".to_owned());
    }

    // SAFETY: Tauri owns this NSWindow for the lifetime of WebviewWindow. This
    // function runs on the app thread and uses only documented AppKit flags.
    let native = unsafe { &*(raw.cast::<NSWindow>()) };
    let behavior = NSWindowCollectionBehavior::CanJoinAllSpaces
        | NSWindowCollectionBehavior::FullScreenAuxiliary;
    native.setCollectionBehavior(behavior);
    Ok(())
}

#[cfg(target_os = "macos")]
mod dock_menu {
    use super::{hide_main_window, show_main_window};
    use objc2::{MainThreadMarker, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSMenu, NSMenuItem};
    use objc2_foundation::NSString;
    use std::{ffi::c_char, sync::OnceLock};
    use tauri::AppHandle;

    static APP_HANDLE: OnceLock<AppHandle> = OnceLock::new();

    #[link(name = "objc", kind = "dylib")]
    unsafe extern "C" {
        fn object_getClass(object: *const AnyObject) -> *mut AnyObject;
        fn sel_registerName(name: *const c_char) -> *const AnyObject;
        fn class_replaceMethod(
            class: *mut AnyObject,
            selector: *const AnyObject,
            implementation: *const AnyObject,
            types: *const c_char,
        ) -> *const AnyObject;
    }

    const DOCK_MENU_SELECTOR: &[u8] = b"applicationDockMenu:\0";
    const SHOW_SELECTOR: &[u8] = b"ccfarmShowMainWindow:\0";
    const QUIT_SELECTOR: &[u8] = b"ccfarmQuitApplication:\0";
    const DOCK_MENU_TYPES: &[u8] = b"@@:@\0";
    const ACTION_TYPES: &[u8] = b"v@:@\0";

    pub fn install(app: AppHandle) -> Result<(), String> {
        APP_HANDLE
            .set(app)
            .map_err(|_| "Dock integration already initialized")?;
        let mtm = MainThreadMarker::new().ok_or("Dock integration requires the main thread")?;
        let application = NSApplication::sharedApplication(mtm);
        let delegate = application
            .delegate()
            .ok_or("NSApplication delegate is unavailable")?;
        let delegate_ptr = (&*delegate as *const _ as *const AnyObject).cast_mut();

        // SAFETY: selectors are registered with NUL-terminated static strings;
        // implementations match the Objective-C method encodings and are added
        // only to the existing app delegate class for this disposable process.
        unsafe {
            let class = object_getClass(delegate_ptr);
            if class.is_null() {
                return Err("NSApplication delegate class is unavailable".to_owned());
            }
            replace(
                class,
                DOCK_MENU_SELECTOR,
                dock_menu as *const AnyObject,
                DOCK_MENU_TYPES,
            );
            replace(class, SHOW_SELECTOR, show as *const AnyObject, ACTION_TYPES);
            replace(class, QUIT_SELECTOR, quit as *const AnyObject, ACTION_TYPES);
        }
        Ok(())
    }

    unsafe fn replace(
        class: *mut AnyObject,
        selector_name: &'static [u8],
        implementation: *const AnyObject,
        types: &'static [u8],
    ) {
        let selector = unsafe { sel_registerName(selector_name.as_ptr().cast()) };
        let _ =
            unsafe { class_replaceMethod(class, selector, implementation, types.as_ptr().cast()) };
    }

    unsafe extern "C-unwind" fn dock_menu(
        target: *mut AnyObject,
        _command: *const AnyObject,
        _application: *mut AnyObject,
    ) -> *mut NSMenu {
        let mtm = unsafe { MainThreadMarker::new_unchecked() };
        let menu = NSMenu::new(mtm);

        let show_item = NSMenuItem::new(mtm);
        show_item.setTitle(&NSString::from_str("显示主画面"));
        unsafe {
            show_item.setTarget(target.as_ref());
            show_item.setAction(Some(objc2::sel!(ccfarmShowMainWindow:)));
        }
        menu.addItem(&show_item);
        menu.addItem(&NSMenuItem::separatorItem(mtm));

        let quit_item = NSMenuItem::new(mtm);
        quit_item.setTitle(&NSString::from_str("退出游戏"));
        unsafe {
            quit_item.setTarget(target.as_ref());
            quit_item.setAction(Some(objc2::sel!(ccfarmQuitApplication:)));
        }
        menu.addItem(&quit_item);

        Retained::autorelease_return(menu)
    }

    unsafe extern "C-unwind" fn show(
        _target: *mut AnyObject,
        _command: *const AnyObject,
        _sender: *mut AnyObject,
    ) {
        if let Some(app) = APP_HANDLE.get() {
            let _ = show_main_window(app);
        }
    }

    unsafe extern "C-unwind" fn quit(
        _target: *mut AnyObject,
        _command: *const AnyObject,
        _sender: *mut AnyObject,
    ) {
        if let Some(app) = APP_HANDLE.get() {
            app.exit(0);
        }
    }

    #[allow(dead_code)]
    fn _hide_for_link_check(app: &AppHandle) {
        let _ = hide_main_window(app);
    }
}

#[cfg(target_os = "macos")]
pub fn install_dock_menu(app: AppHandle) -> Result<(), String> {
    dock_menu::install(app)
}

#[cfg(not(target_os = "macos"))]
pub fn install_dock_menu(_app: AppHandle) -> Result<(), String> {
    Ok(())
}
