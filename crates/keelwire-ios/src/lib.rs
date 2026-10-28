//! iOS / iPadOS glue, called on the main thread by the app (see
//! `keelwire-tauri`): keep the screen awake while a session records, and
//! present the system share sheet for saved reports.
//!
//! All functions are no-ops on other platforms.

use std::path::PathBuf;

/// Keep the screen on (idle timer disabled) or restore auto-lock.
/// Must be called on the main thread.
pub fn set_keep_awake(on: bool) {
    #[cfg(target_os = "ios")]
    imp::set_keep_awake(on);
    #[cfg(not(target_os = "ios"))]
    let _ = on;
}

/// Present the share sheet (AirDrop, Mail, Save to Files, Print…) for files.
/// Must be called on the main thread.
pub fn share_files(paths: &[PathBuf]) -> Result<(), String> {
    #[cfg(target_os = "ios")]
    return imp::share_files(paths);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = paths;
        Err("sharing is only available on iPhone and iPad".into())
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use std::path::PathBuf;

    use objc2::rc::Retained;
    use objc2::MainThreadMarker;
    use objc2_core_foundation::{CGPoint, CGRect, CGSize};
    use objc2_foundation::{NSArray, NSString, NSURL};
    use objc2_ui_kit::{UIActivityViewController, UIApplication, UIPopoverArrowDirection, UIViewController};

    pub fn set_keep_awake(on: bool) {
        if let Some(mtm) = MainThreadMarker::new() {
            UIApplication::sharedApplication(mtm).setIdleTimerDisabled(on);
        }
    }

    /// The view controller currently on screen (the web view's, or whatever it presents).
    fn top_controller(mtm: MainThreadMarker) -> Option<Retained<UIViewController>> {
        let app = UIApplication::sharedApplication(mtm);
        #[allow(deprecated)] // single-scene app: the key window is ours
        let window = app.keyWindow()?;
        let mut vc = window.rootViewController()?;
        while let Some(p) = vc.presentedViewController() {
            vc = p;
        }
        Some(vc)
    }

    pub fn share_files(paths: &[PathBuf]) -> Result<(), String> {
        let mtm = MainThreadMarker::new().ok_or("share_files must run on the main thread")?;
        let urls: Vec<Retained<NSURL>> = paths.iter().filter_map(|p| p.to_str()).map(|p| NSURL::fileURLWithPath(&NSString::from_str(p))).collect();
        if urls.is_empty() {
            return Err("nothing to share".into());
        }
        // NSArray<NSURL> → NSArray<AnyObject> (the untyped array the API takes).
        // SAFETY: an array of NSURL is an array of objects.
        let items: Retained<NSArray> = unsafe { Retained::cast_unchecked(NSArray::from_retained_slice(&urls)) };
        let vc = top_controller(mtm).ok_or("no window to present from")?;
        // SAFETY: the activity items are file NSURLs, which the share sheet accepts.
        let sheet = unsafe { UIActivityViewController::initWithActivityItems_applicationActivities(mtm.alloc(), &items, None) };
        // iPad presents the sheet as a popover, which needs an anchor; without
        // one UIKit raises an exception. Anchor it near the top-right corner,
        // where the session controls are.
        if let (Some(pop), Some(view)) = (sheet.popoverPresentationController(), vc.view()) {
            let b = view.bounds();
            pop.setSourceView(Some(&view));
            pop.setSourceRect(CGRect { origin: CGPoint { x: b.size.width - 60.0, y: 70.0 }, size: CGSize { width: 1.0, height: 1.0 } });
            pop.setPermittedArrowDirections(UIPopoverArrowDirection::Up);
        }
        vc.presentViewController_animated_completion(&sheet, true, None);
        Ok(())
    }
}
