#![allow(dead_code)]
#![allow(non_snake_case)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use reqwest::Client;

const CLIENT_ID: &str = "00000000402B5328"; // Official Minecraft Launcher Client ID

// --- Mojang Version Manifest Structs ---

#[derive(Deserialize, Debug, Clone)]
pub struct LatestVersion {
    pub release: String,
    pub snapshot: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct VersionBrief {
    pub id: String,
    pub r#type: String,
    pub url: String,
    pub time: String,
    pub releaseTime: String,
    pub sha1: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct VersionManifest {
    pub latest: LatestVersion,
    pub versions: Vec<VersionBrief>,
}

// --- Minecraft Version Details Structs ---

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RuleOS {
    pub name: Option<String>,
    pub arch: Option<Option<String>>, // Can be nested or simple
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Rule {
    pub action: String,
    pub os: Option<RuleOS>,
}

impl Rule {
    pub fn matches_current_env(&self) -> bool {
        if let Some(ref os_rule) = self.os {
            if let Some(ref name) = os_rule.name {
                let current_os = if cfg!(target_os = "windows") {
                    "windows"
                } else if cfg!(target_os = "macos") {
                    "osx"
                } else if cfg!(target_os = "linux") {
                    "linux"
                } else {
                    "unknown"
                };
                if name != current_os {
                    return false;
                }
            }
            if let Some(arch_opt) = &os_rule.arch {
                if let Some(arch) = arch_opt {
                    let current_arch = if cfg!(target_arch = "x86") {
                        "x86"
                    } else if cfg!(target_arch = "x86_64") {
                        "x64"
                    } else {
                        "unknown"
                    };
                    if arch != current_arch {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn evaluate(rules: &[Rule]) -> bool {
        if rules.is_empty() {
            return true;
        }
        let mut allowed = false;
        for rule in rules {
            if rule.matches_current_env() {
                allowed = rule.action == "allow";
            }
        }
        allowed
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum ArgumentValueList {
    Single(String),
    Many(Vec<String>),
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(untagged)]
pub enum ArgumentValue {
    Simple(String),
    Complex {
        rules: Vec<Rule>,
        value: ArgumentValueList,
    },
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Arguments {
    pub game: Vec<ArgumentValue>,
    pub jvm: Vec<ArgumentValue>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Artifact {
    pub path: String,
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct LibraryDownloads {
    pub artifact: Option<Artifact>,
    pub classifiers: Option<HashMap<String, Artifact>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Library {
    pub name: String,
    pub downloads: LibraryDownloads,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<HashMap<String, String>>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct AssetIndexRef {
    pub id: String,
    pub sha1: String,
    pub size: u64,
    pub totalSize: u64,
    pub url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DownloadsRef {
    pub client: Artifact,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct JavaVersion {
    pub component: String,
    pub majorVersion: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetails {
    pub id: String,
    pub r#type: String,
    pub mainClass: String,
    pub arguments: Option<Arguments>,
    pub minecraftArguments: Option<String>,
    pub libraries: Vec<Library>,
    pub assetIndex: AssetIndexRef,
    pub downloads: DownloadsRef,
    pub javaVersion: Option<JavaVersion>,
}

// --- Asset Index Structures ---

#[derive(Deserialize, Debug, Clone)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct AssetIndex {
    pub objects: HashMap<String, AssetObject>,
}

// --- Microsoft Auth Flow Structs ---

#[derive(Deserialize, Debug, Clone)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub expires_in: u64,
    pub interval: u64,
    pub message: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_in: u64,
}

#[derive(Serialize)]
struct XboxLiveProperties {
    #[serde(rename = "AuthMethod")]
    auth_method: String,
    #[serde(rename = "SiteName")]
    site_name: String,
    #[serde(rename = "RpsTicket")]
    rps_ticket: String,
}

#[derive(Serialize)]
struct XboxLivePayload {
    #[serde(rename = "Properties")]
    properties: XboxLiveProperties,
    #[serde(rename = "RelyingParty")]
    relying_party: String,
    #[serde(rename = "TokenType")]
    token_type: String,
}

#[derive(Deserialize, Debug, Clone)]
struct XuiClaim {
    uhs: String,
}

#[derive(Deserialize, Debug, Clone)]
struct DisplayClaims {
    xui: Vec<XuiClaim>,
}

#[derive(Deserialize, Debug, Clone)]
struct XboxLiveResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Serialize)]
struct XstsProperties {
    #[serde(rename = "SandboxId")]
    sandbox_id: String,
    #[serde(rename = "UserTokens")]
    user_tokens: Vec<String>,
}

#[derive(Serialize)]
struct XstsPayload {
    #[serde(rename = "Properties")]
    properties: XstsProperties,
    #[serde(rename = "RelyingParty")]
    relying_party: String,
    #[serde(rename = "TokenType")]
    token_type: String,
}

#[derive(Serialize)]
struct MinecraftLoginPayload {
    #[serde(rename = "identityToken")]
    identity_token: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MinecraftLoginResponse {
    pub access_token: String,
    pub expires_in: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct MinecraftProfile {
    pub id: String,
    pub name: String,
}

// --- API Client ---

pub struct ApiClient {
    client: Client,
}

impl ApiClient {
    pub fn new() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap(),
        }
    }

    pub async fn fetch_version_manifest(&self) -> Result<VersionManifest, String> {
        let url = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";
        self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch version manifest: {}", e))?
            .json::<VersionManifest>()
            .await
            .map_err(|e| format!("Failed to parse version manifest: {}", e))
    }

    pub async fn fetch_version_details(&self, url: &str) -> Result<VersionDetails, String> {
        self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch version details: {}", e))?
            .json::<VersionDetails>()
            .await
            .map_err(|e| format!("Failed to parse version details: {}", e))
    }

    pub async fn fetch_asset_index(&self, url: &str) -> Result<AssetIndex, String> {
        self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch asset index: {}", e))?
            .json::<AssetIndex>()
            .await
            .map_err(|e| format!("Failed to parse asset index: {}", e))
    }

    // --- MS Login Flow ---

    pub async fn request_device_code(&self) -> Result<DeviceCodeResponse, String> {
        let url = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
        let params = [
            ("client_id", CLIENT_ID),
            ("scope", "XboxLive.signin offline_access"),
        ];

        self.client.post(url)
            .form(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json::<DeviceCodeResponse>()
            .await
            .map_err(|e| e.to_string())
    }

    pub async fn poll_token(&self, device_code: &str) -> Result<Option<TokenResponse>, String> {
        let url = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
        let params = [
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
            ("device_code", device_code),
            ("client_id", CLIENT_ID),
        ];

        let res = self.client.post(url)
            .form(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        let status = res.status();
        let body = res.text().await.map_err(|e| e.to_string())?;

        if status.is_success() {
            let token_res = serde_json::from_str::<TokenResponse>(&body).map_err(|e| e.to_string())?;
            Ok(Some(token_res))
        } else {
            // Check for pending authorization error
            if body.contains("authorization_pending") {
                Ok(None)
            } else {
                Err(format!("Error polling token: {}", body))
            }
        }
    }

    pub async fn refresh_token(&self, refresh_token: &str) -> Result<TokenResponse, String> {
        let url = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
        let params = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", CLIENT_ID),
        ];

        self.client.post(url)
            .form(&params)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .json::<TokenResponse>()
            .await
            .map_err(|e| format!("Failed to refresh MS token: {}", e))
    }

    pub async fn login_with_microsoft(&self, ms_access_token: &str) -> Result<MinecraftLoginResponse, String> {
        // Step 1: Authenticate with Xbox Live
        let xbl_url = "https://user.auth.xboxlive.com/user/authenticate";
        let xbl_payload = XboxLivePayload {
            properties: XboxLiveProperties {
                auth_method: "RPS".to_string(),
                site_name: "user.auth.xboxlive.com".to_string(),
                rps_ticket: format!("d={}", ms_access_token),
            },
            relying_party: "http://auth.xboxlive.com".to_string(),
            token_type: "JWT".to_string(),
        };

        let xbl_res = self.client.post(xbl_url)
            .json(&xbl_payload)
            .send()
            .await
            .map_err(|e| format!("Xbox Live auth failed: {}", e))?
            .json::<XboxLiveResponse>()
            .await
            .map_err(|e| format!("Failed to parse Xbox Live response: {}", e))?;

        let user_hash = xbl_res.display_claims.xui.first()
            .ok_or("No user hash found in Xbox Live response")?
            .uhs.clone();

        // Step 2: Request XSTS Token
        let xsts_url = "https://xsts.auth.xboxlive.com/xsts/authorize";
        let xsts_payload = XstsPayload {
            properties: XstsProperties {
                sandbox_id: "RETAIL".to_string(),
                user_tokens: vec![xbl_res.token],
            },
            relying_party: "rp://api.minecraftservices.com/".to_string(),
            token_type: "JWT".to_string(),
        };

        let xsts_res = self.client.post(xsts_url)
            .json(&xsts_payload)
            .send()
            .await
            .map_err(|e| format!("XSTS auth failed: {}", e))?;

        if !xsts_res.status().is_success() {
            let err_body = xsts_res.text().await.unwrap_or_default();
            if err_body.contains("2148916233") {
                return Err("Microsoft account does not have an Xbox Live profile. Please create one on xbox.com.".to_string());
            } else if err_body.contains("2148916238") {
                return Err("Account belongs to a minor and requires parental consent on Xbox.".to_string());
            }
            return Err(format!("XSTS token request failed: {}", err_body));
        }

        let xsts_res_parsed = xsts_res.json::<XboxLiveResponse>()
            .await
            .map_err(|e| format!("Failed to parse XSTS response: {}", e))?;

        // Step 3: Login to Minecraft
        let mc_url = "https://api.minecraftservices.com/authentication/login_with_xbox";
        let mc_payload = MinecraftLoginPayload {
            identity_token: format!("XBL3.0 x={};{}", user_hash, xsts_res_parsed.token),
        };

        self.client.post(mc_url)
            .json(&mc_payload)
            .send()
            .await
            .map_err(|e| format!("Minecraft login request failed: {}", e))?
            .json::<MinecraftLoginResponse>()
            .await
            .map_err(|e| format!("Failed to parse Minecraft login response: {}", e))
    }

    pub async fn fetch_profile(&self, mc_access_token: &str) -> Result<MinecraftProfile, String> {
        let url = "https://api.minecraftservices.com/minecraft/profile";
        let res = self.client.get(url)
            .header("Authorization", format!("Bearer {}", mc_access_token))
            .send()
            .await
            .map_err(|e| format!("Fetch profile failed: {}", e))?;

        if res.status() == 404 {
            return Err("User does not own Minecraft Java Edition on this account.".to_string());
        }

        res.json::<MinecraftProfile>()
            .await
            .map_err(|e| format!("Failed to parse profile response: {}", e))
    }
}
