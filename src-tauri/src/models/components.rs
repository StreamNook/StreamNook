use serde::{Deserialize, Serialize};

/// Schema version for components.json format
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Component manifest that tracks bundled component versions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentManifest {
    pub schema_version: u32,
    pub streamnook: StreamNookInfo,
    pub components: Components,
}

/// StreamNook app info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamNookInfo {
    pub version: String,
    pub build_date: String,
}

/// Bundled components info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Components {
    pub streamlink: ComponentInfo,
    pub ttvlol: ComponentInfo,
}

/// Individual component info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentInfo {
    pub version: String,
    pub source_url: String,
}

/// Update status returned to frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleUpdateStatus {
    pub update_available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub download_url: Option<String>,
    pub bundle_name: Option<String>,
    pub download_size: Option<String>,
    pub component_changes: Option<ComponentChanges>,
    pub release_notes: Option<String>,
    /// Expected SHA-256 of the bundle, when the source advertises one (the
    /// streamnook.app manifest does). The installer verifies the download
    /// against this before swapping the exe. None = no integrity check.
    #[serde(default)]
    pub sha256: Option<String>,
    /// Detached minisign signature over the bundle bytes, as the text content of
    /// a `.minisig` file.
    ///
    /// This is the only integrity control that survives a compromise of the
    /// update host. `sha256` proves the bytes match what the manifest CLAIMS,
    /// but both come from the same server, so whoever can serve a malicious
    /// bundle can serve a matching hash. The signing key lives only in CI, so a
    /// signature cannot be forged by anyone who merely controls the host.
    #[serde(default)]
    pub signature: Option<String>,
}

/// Details about which components changed
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentChanges {
    pub streamnook: Option<VersionChange>,
    pub streamlink: Option<VersionChange>,
    pub ttvlol: Option<VersionChange>,
}

/// Version change info
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionChange {
    pub from: String,
    pub to: String,
}

impl ComponentManifest {
    /// Create a new manifest with default values
    pub fn new(streamnook_version: &str) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION,
            streamnook: StreamNookInfo {
                version: streamnook_version.to_string(),
                build_date: chrono::Utc::now().to_rfc3339(),
            },
            components: Components {
                streamlink: ComponentInfo {
                    version: String::new(),
                    source_url: "https://github.com/streamlink/windows-builds/releases".to_string(),
                },
                ttvlol: ComponentInfo {
                    version: String::new(),
                    source_url: "https://github.com/2bc4/streamlink-ttvlol/releases".to_string(),
                },
            },
        }
    }

    /// Load manifest from a file path
    pub fn load_from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let manifest: ComponentManifest = serde_json::from_str(&content)?;
        Ok(manifest)
    }

    /// Save manifest to a file path
    pub fn save_to_file(&self, path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    /// Compare two manifests and determine if an update is needed
    pub fn compare(&self, remote: &ComponentManifest) -> BundleUpdateStatus {
        let streamnook_changed = self.streamnook.version != remote.streamnook.version;
        let streamlink_changed =
            self.components.streamlink.version != remote.components.streamlink.version;
        let ttvlol_changed = self.components.ttvlol.version != remote.components.ttvlol.version;

        let update_available = streamnook_changed || streamlink_changed || ttvlol_changed;

        let component_changes = if update_available {
            Some(ComponentChanges {
                streamnook: if streamnook_changed {
                    Some(VersionChange {
                        from: self.streamnook.version.clone(),
                        to: remote.streamnook.version.clone(),
                    })
                } else {
                    None
                },
                streamlink: if streamlink_changed {
                    Some(VersionChange {
                        from: self.components.streamlink.version.clone(),
                        to: remote.components.streamlink.version.clone(),
                    })
                } else {
                    None
                },
                ttvlol: if ttvlol_changed {
                    Some(VersionChange {
                        from: self.components.ttvlol.version.clone(),
                        to: remote.components.ttvlol.version.clone(),
                    })
                } else {
                    None
                },
            })
        } else {
            None
        };

        BundleUpdateStatus {
            update_available,
            current_version: self.streamnook.version.clone(),
            latest_version: remote.streamnook.version.clone(),
            download_url: None,
            bundle_name: None,
            download_size: None,
            component_changes,
            release_notes: None,
            sha256: None,
            signature: None,
        }
    }
}

impl Default for ComponentManifest {
    fn default() -> Self {
        Self::new("0.0.0")
    }
}
