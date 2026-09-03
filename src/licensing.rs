use std::fmt::Write as _;
use std::path::Path;
use std::sync::Arc;

use derpcat_access::{
    AccessBootstrap, AccessError, FeatureGate, LicenseChannel, ProductAccess, ProductConfig,
    PublicKey, features,
};

pub(crate) use derpcat_access::LicenseState;

const BETA_LICENSE_PUBLIC_KEY: PublicKey = PublicKey {
    key_id: 1,
    bytes: [
        0xdd, 0x88, 0xc4, 0x52, 0x2f, 0x8d, 0x8e, 0x1f, 0xad, 0xbe, 0x4e, 0xaa, 0xff, 0xf5, 0x77,
        0x19, 0x77, 0x38, 0x65, 0x66, 0xab, 0x17, 0x0b, 0xdd, 0xb7, 0xd9, 0x34, 0x53, 0xf8, 0xfc,
        0xfe, 0x1e,
    ],
};
const FULL_LICENSE_PUBLIC_KEY: PublicKey = PublicKey {
    key_id: 2,
    bytes: [
        0xfc, 0x94, 0xec, 0x54, 0xf6, 0xe3, 0x40, 0x65, 0x72, 0xff, 0xc0, 0xcc, 0xb5, 0x2b, 0xe2,
        0xa7, 0x4d, 0x98, 0x3e, 0xae, 0xac, 0x91, 0x25, 0xa5, 0x64, 0x32, 0xc2, 0xca, 0x26, 0xb9,
        0x88, 0xd0,
    ],
};
const RUNTIME_PUBLIC_KEY: PublicKey = PublicKey {
    key_id: 1,
    bytes: [
        0xc0, 0x0b, 0xe6, 0x75, 0x39, 0x79, 0xad, 0x14, 0x33, 0x18, 0xd5, 0xe2, 0xb3, 0x1f, 0xf3,
        0xdb, 0x74, 0x1d, 0x51, 0x88, 0xf5, 0xff, 0x24, 0xee, 0xd9, 0x08, 0x49, 0x07, 0x83, 0xdd,
        0x00, 0x2b,
    ],
};
const REALTIME_GATE: FeatureGate = FeatureGate::from_parts(
    features::FULL,
    0x6b75_7276_2d72_7431,
    0x6b75_7276_2d6d_6f64,
    0x6b75_7276_2d74_6167,
);

pub(crate) struct PluginActivation {
    access: Arc<ProductAccess>,
    _bootstrap: AccessBootstrap,
    realtime_gate: FeatureGate,
}

impl Default for PluginActivation {
    fn default() -> Self {
        let config = ProductConfig::new("kurv", env!("CARGO_PKG_VERSION"))
            .build_family("kurv-beta-v1")
            .channel(LicenseChannel::Beta)
            .license_keys(vec![BETA_LICENSE_PUBLIC_KEY, FULL_LICENSE_PUBLIC_KEY])
            .runtime_keys(vec![RUNTIME_PUBLIC_KEY])
            .trial_features(features::FULL)
            .trial_activation_id_prefix("trial.kurv.")
            .user_agent(format!("KURV/{}", env!("CARGO_PKG_VERSION")));
        let (access, bootstrap) = ProductAccess::start(config);
        let _ = access.start_trial();
        Self {
            access,
            _bootstrap: bootstrap,
            realtime_gate: REALTIME_GATE,
        }
    }
}

impl PluginActivation {
    pub(crate) fn subscribe_changes(&self, listener: Arc<dyn Fn() + Send + Sync>) {
        self.access.subscribe_changes(listener);
    }

    pub(crate) fn load_installed_license(&self) -> Result<Option<String>, String> {
        self.access.load_installed_license().map_err(access_error)
    }

    pub(crate) fn import_license_file(&self, path: &Path) -> Result<String, String> {
        self.access.import_license_file(path).map_err(access_error)
    }

    pub(crate) fn activation_request(&self) -> Result<String, String> {
        self.access.activation_request().map_err(access_error)
    }

    pub(crate) fn start_trial(&self) -> Result<(), String> {
        self.access.start_trial().map_err(access_error)
    }

    pub(crate) fn license_state(&self) -> LicenseState {
        self.access.license_state()
    }

    #[inline]
    pub(crate) fn features_enabled(&self) -> bool {
        #[cfg(any(test, feature = "process-lab"))]
        return true;
        #[cfg(not(any(test, feature = "process-lab")))]
        self.access.can_open(&self.realtime_gate)
    }

    pub(crate) fn access_check_pending(&self) -> bool {
        self.access.access_check_pending()
    }

    pub(crate) fn machine_id_hex(&self) -> Option<String> {
        self.access
            .support_machine_hashes()
            .into_iter()
            .next()
            .map(|id| {
                id.iter().fold(String::with_capacity(64), |mut hex, byte| {
                    let _ = write!(hex, "{byte:02x}");
                    hex
                })
            })
    }
}

fn access_error(error: AccessError) -> String {
    error.to_string()
}
