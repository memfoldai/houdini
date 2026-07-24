use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{class, msg_send};
use objc2_foundation::NSString;
use std::path::Path;

#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}

const STATUS_NOT_REGISTERED: isize = 0;
const STATUS_ENABLED: isize = 1;
const STATUS_REQUIRES_APPROVAL: isize = 2;
const STATUS_NOT_FOUND: isize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginItem {
    Enabled,
    NotRegistered,
    RequiresApproval,
    NotFound,
    Unavailable,
}

impl LoginItem {
    fn from_raw(raw: isize) -> Self {
        match raw {
            STATUS_ENABLED => LoginItem::Enabled,
            STATUS_NOT_REGISTERED => LoginItem::NotRegistered,
            STATUS_REQUIRES_APPROVAL => LoginItem::RequiresApproval,
            STATUS_NOT_FOUND => LoginItem::NotFound,
            _ => LoginItem::Unavailable,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            LoginItem::Enabled => "starts automatically at login",
            LoginItem::NotRegistered => "not set to start at login",
            LoginItem::RequiresApproval => {
                "waiting for approval in System Settings > General > Login Items"
            }
            LoginItem::NotFound => "login item registration is missing",
            LoginItem::Unavailable => "login item state is unavailable",
        }
    }
}

fn service() -> Option<Retained<AnyObject>> {
    let class: &AnyClass = class!(SMAppService);
    unsafe { msg_send![class, mainAppService] }
}

pub fn status() -> LoginItem {
    let Some(service) = service() else {
        return LoginItem::Unavailable;
    };
    let raw: isize = unsafe { msg_send![&*service, status] };
    LoginItem::from_raw(raw)
}

/// Registers the app to launch at login, once, and only for an installed copy.
///
/// A development build lives outside /Applications and would register that
/// throwaway path, so it is skipped for the same reason the updater is. macOS
/// surfaces the registration in System Settings > General > Login Items, which
/// is where someone turns it back off; that is deliberately the only switch, so
/// this app never disagrees with what the system shows.
pub fn ensure_registered(bundle_path: &Path) {
    if !bundle_path.starts_with("/Applications") {
        log::info!("login item: skipped for a build outside /Applications");
        return;
    }
    match status() {
        LoginItem::Enabled => return,
        LoginItem::RequiresApproval => {
            log::info!("login item: {}", LoginItem::RequiresApproval.describe());
            return;
        }
        LoginItem::Unavailable => {
            log::warn!("login item: ServiceManagement is unavailable on this system");
            return;
        }
        LoginItem::NotRegistered | LoginItem::NotFound => {}
    }

    let Some(service) = service() else {
        return;
    };
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let ok: bool = unsafe { msg_send![&*service, registerAndReturnError: &mut error] };
    if ok {
        log::info!("login item: registered, Houdini will start at login");
        return;
    }
    log::warn!("login item: could not register ({})", error_text(error));
}

pub fn unregister() {
    let Some(service) = service() else {
        return;
    };
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let ok: bool = unsafe { msg_send![&*service, unregisterAndReturnError: &mut error] };
    if ok {
        log::info!("login item: unregistered");
    } else {
        log::warn!("login item: could not unregister ({})", error_text(error));
    }
}

fn error_text(error: *mut AnyObject) -> String {
    if error.is_null() {
        return "no error reported".to_string();
    }
    let description: Option<Retained<NSString>> = unsafe { msg_send![error, localizedDescription] };
    description
        .map(|d| d.to_string())
        .unwrap_or_else(|| "unknown error".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_documented_status_maps_to_a_variant() {
        assert_eq!(LoginItem::from_raw(0), LoginItem::NotRegistered);
        assert_eq!(LoginItem::from_raw(1), LoginItem::Enabled);
        assert_eq!(LoginItem::from_raw(2), LoginItem::RequiresApproval);
        assert_eq!(LoginItem::from_raw(3), LoginItem::NotFound);
    }

    #[test]
    fn an_unknown_status_degrades_instead_of_panicking() {
        assert_eq!(LoginItem::from_raw(99), LoginItem::Unavailable);
        assert_eq!(LoginItem::from_raw(-1), LoginItem::Unavailable);
    }

    #[test]
    fn every_state_reads_as_plain_english() {
        for state in [
            LoginItem::Enabled,
            LoginItem::NotRegistered,
            LoginItem::RequiresApproval,
            LoginItem::NotFound,
            LoginItem::Unavailable,
        ] {
            assert!(!state.describe().is_empty());
        }
    }

    #[test]
    fn a_dev_build_never_registers_its_throwaway_path() {
        ensure_registered(Path::new("/Users/someone/code/target/release/Houdini.app"));
        assert_ne!(
            status(),
            LoginItem::Enabled,
            "a path outside /Applications must not become a login item"
        );
    }
}
