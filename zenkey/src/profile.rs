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
//! static PROFILE: AppProfile = AppProfile::new("acme-fleet", "acme-fleet-host-id-v1");
//! ```
//!
//! and passes it to [`crate::V1Context::for_producer`]. The deployment *base*
//! is deliberately not part of the profile: it is session configuration
//! (the Zenoh namespace, RFC 03 §1.1), not an application constant.

use std::path::PathBuf;
use std::sync::OnceLock;

use crate::origin::HostId;

/// An application's identity constants plus its process-wide host origin.
///
/// One per application, one per process (the host id is minted once and
/// cached; two profiles in one process each mint independently — coherent,
/// but almost never what you want).
#[derive(Debug)]
pub struct AppProfile {
    app: &'static str,
    salt: &'static str,
    host_id: OnceLock<HostId>,
}

impl AppProfile {
    /// `app` names the application (it must be a valid plain chunk, RFC 03 §2 —
    /// it becomes a directory name in the host-id fallback path). `salt` is the
    /// RFC 06 §1 application salt mixed into origin minting.
    pub const fn new(app: &'static str, salt: &'static str) -> Self {
        AppProfile {
            app,
            salt,
            host_id: OnceLock::new(),
        }
    }

    pub fn app(&self) -> &'static str {
        self.app
    }

    pub fn salt(&self) -> &'static str {
        self.salt
    }

    /// Where a host id is persisted when `/etc/machine-id` is unusable
    /// (RFC 06 §1.1): `$XDG_STATE_HOME/<app>/host-id`, else
    /// `/var/lib/<app>/host-id`.
    pub fn host_id_fallback_path(&self) -> PathBuf {
        dirs::state_dir()
            .map(|d| d.join(self.app).join("host-id"))
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
            tracing::info!(origin = %id, app = self.app, "host origin minted");
            id
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_path_is_app_derived() {
        let p = AppProfile::new("acme-fleet", "s");
        let path = p.host_id_fallback_path();
        assert!(
            path.ends_with("acme-fleet/host-id"),
            "unexpected fallback path: {path:?}"
        );
    }

    #[test]
    fn host_id_is_minted_once() {
        let p = AppProfile::new("zenkey-profile-test", "test-salt-v1");
        let a = p.host_id().clone();
        let b = p.host_id().clone();
        assert_eq!(a, b);
    }
}
