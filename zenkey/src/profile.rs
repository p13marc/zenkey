//! The application profile: the two constants an adopting application must
//! choose, bundled with the once-per-process host-origin mint (RFC 06 §1,
//! RFC 11 §4).
//!
//! The convention is application-neutral; what makes a concrete fleet is a
//! *profile* — an application name and an origin salt. Both are **application
//! constants**: compiled in, not operator-configurable, identical across
//! deployments of the same application. Changing the salt re-keys every fleet.
//!
//! An application declares exactly one profile, as a static:
//!
//! ```
//! use zenkey::AppProfile;
//!
//! use zenkey::{AppName, OriginSalt};
//! static PROFILE: AppProfile = AppProfile::new(
//!     AppName::new("acme-fleet"),
//!     OriginSalt::new("acme-fleet-host-id-v1"),
//! );
//! ```
//!
//! and passes it to [`crate::V1Context::for_producer`]. The deployment *base*
//! is deliberately not part of the profile: it is session configuration
//! (the Zenoh namespace, RFC 03 §1.1), not an application constant.

use std::fmt;
use std::path::PathBuf;
use std::sync::OnceLock;

use crate::origin::{HostId, LocalOrigin};

/// An application's name — the plain chunk that becomes a directory name in
/// the host-id fallback path (RFC 06 §1.1).
///
/// A newtype because the two constants of a profile are both `&'static str`
/// and sit next to each other: `AppProfile::new("acme-fleet-host-id-v1",
/// "acme-fleet")` used to compile, build a working profile with a wrong
/// host-id path *and* a wrong salt, and be detectable only by noticing that
/// every origin in the fleet had changed (#324). Naming each half at the call
/// site is what makes that mistake visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AppName(&'static str);

impl AppName {
    /// Validated at **compile time**: this is a `const fn` and an application
    /// declares its profile as a `static`, so an illegal name is a build
    /// error rather than a path that misbehaves at runtime.
    pub const fn new(name: &'static str) -> Self {
        assert!(
            crate::grammar::is_valid_plain_chunk(name),
            "an application name must be a valid plain chunk (RFC 03 §2) — it becomes a directory \
             name in the host-id fallback path"
        );
        AppName(name)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for AppName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The RFC 06 §1 application salt mixed into every origin this fleet mints.
///
/// The sibling of [`AppName`], and the more dangerous of the two: changing it
/// re-keys every origin in the fleet, so a value that arrived here by
/// transposition is a silent fleet-wide re-key. There is no charset rule to
/// check — a salt is hash input — so the constructor checks the one thing
/// that is always wrong, and the *type* carries the rest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OriginSalt(&'static str);

impl OriginSalt {
    /// Compile-time checked, like [`AppName::new`].
    pub const fn new(salt: &'static str) -> Self {
        assert!(
            !salt.is_empty(),
            "an origin salt must not be empty (RFC 06 §1)"
        );
        OriginSalt(salt)
    }

    pub const fn as_str(&self) -> &'static str {
        self.0
    }
}

impl fmt::Display for OriginSalt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// An application's identity constants plus its process-wide host origin.
///
/// One per application, one per process (the host id is minted once and
/// cached; two profiles in one process each mint independently — coherent,
/// but almost never what you want).
#[derive(Debug)]
pub struct AppProfile {
    app: AppName,
    salt: OriginSalt,
    host_id: OnceLock<HostId>,
}

impl AppProfile {
    /// `app` names the application; `salt` is the RFC 06 §1 application salt
    /// mixed into origin minting. Both are checked where they are spelled —
    /// see [`AppName::new`] and [`OriginSalt::new`].
    pub const fn new(app: AppName, salt: OriginSalt) -> Self {
        AppProfile {
            app,
            salt,
            host_id: OnceLock::new(),
        }
    }

    pub fn app(&self) -> AppName {
        self.app
    }

    pub fn salt(&self) -> OriginSalt {
        self.salt
    }

    /// Where a host id is persisted when `/etc/machine-id` is unusable
    /// (RFC 06 §1.1): `$XDG_STATE_HOME/<app>/host-id`, else
    /// `/var/lib/<app>/host-id`.
    pub fn host_id_fallback_path(&self) -> PathBuf {
        dirs::state_dir()
            .map(|d| d.join(self.app.as_str()).join("host-id"))
            .unwrap_or_else(|| PathBuf::from(format!("/var/lib/{}/host-id", self.app)))
    }

    /// The process-wide host origin (RFC 06 §1): `/etc/machine-id` + the
    /// application salt, with the persisted-random fallback. Minted once per
    /// profile, then cached.
    pub fn host_id(&self) -> &HostId {
        self.host_id.get_or_init(|| {
            let id = HostId::mint(
                std::path::Path::new("/etc/machine-id"),
                &self.host_id_fallback_path(),
                self.salt,
            );
            tracing::info!(origin = %id, app = %self.app, "host origin minted");
            id
        })
    }

    /// This process's typed local origin (RFC 08 §1.1) — the only
    /// non-explicit constructor of [`LocalOrigin`], so a call path cannot
    /// mint one by accident.
    pub fn local_origin(&'static self) -> LocalOrigin {
        LocalOrigin::from_host_id(self.host_id().clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The validation is **compile-time**, and this is what proves it: a
    /// `const` item is evaluated by the compiler, so if `AppName::new`'s
    /// `assert!` were reachable at runtime only, this would not be a `const`.
    ///
    /// The negative case cannot be written as a test — it is a build failure
    /// by design. `const _: AppName = AppName::new("Acme Fleet");` does not
    /// compile (capitals and a space are not plain-chunk legal, RFC 03 §2),
    /// and neither does `OriginSalt::new("")`.
    #[test]
    fn the_profile_constants_are_checked_at_compile_time() {
        const APP: AppName = AppName::new("acme-fleet");
        const SALT: OriginSalt = OriginSalt::new("acme-fleet-host-id-v1");
        // A profile is a `static`, never a `const` — it caches its minted host
        // id in a `OnceLock`, and a `const` would hand every use site its own
        // copy of that cache. The two constants above are the compile-time
        // half; this is how an application spells the whole thing.
        static PROFILE: AppProfile = AppProfile::new(APP, SALT);

        assert_eq!(APP.as_str(), "acme-fleet");
        assert_eq!(SALT.as_str(), "acme-fleet-host-id-v1");
        assert_eq!(PROFILE.app(), APP);
        assert_eq!(PROFILE.salt(), SALT);
    }

    #[test]
    fn fallback_path_is_app_derived() {
        let p = AppProfile::new(AppName::new("acme-fleet"), OriginSalt::new("s"));
        let path = p.host_id_fallback_path();
        assert!(
            path.ends_with("acme-fleet/host-id"),
            "unexpected fallback path: {path:?}"
        );
    }

    #[test]
    fn host_id_is_minted_once() {
        let p = AppProfile::new(
            AppName::new("zenkey-profile-test"),
            OriginSalt::new("test-salt-v1"),
        );
        let a = p.host_id().clone();
        let b = p.host_id().clone();
        assert_eq!(a, b);
    }
}
