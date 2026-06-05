#![allow(dead_code)]
#![allow(non_snake_case)]

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use reqwest::Client;

const CLIENT_ID: &str = "c36a9fb6-4f2a-41ff-90bd-ae7cc92031eb"; // Prism Launcher Client ID (allows Device Code OAuth)

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
pub struct RuleFeatures {
    #[serde(rename = "is_demo_user")]
    pub is_demo_user: Option<bool>,
    #[serde(rename = "has_custom_resolution")]
    pub has_custom_resolution: Option<bool>,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Rule {
    pub action: String,
    pub os: Option<RuleOS>,
    pub features: Option<RuleFeatures>,
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
            if let Some(arch_opt) = &os_rule.arch
                && let Some(arch) = arch_opt {
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
        if let Some(ref features_rule) = self.features {
            if let Some(is_demo) = features_rule.is_demo_user
                && is_demo {
                    return false;
                }
            if let Some(has_custom_res) = features_rule.has_custom_resolution
                && !has_custom_res {
                    return false;
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

pub fn maven_to_path(name: &str) -> Option<String> {
    let parts: Vec<&str> = name.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    
    let (classifier, ext) = if parts.len() == 4 {
        let last = parts[3];
        if last.contains('@') {
            let subparts: Vec<&str> = last.split('@').collect();
            (Some(subparts[0]), subparts[1])
        } else {
            (Some(last), "jar")
        }
    } else if parts.len() == 3 {
        if version.contains('@') {
            let subparts: Vec<&str> = version.split('@').collect();
            let clean_version = subparts[0];
            return Some(format!("{}/{}/{}/{}-{}.{}", group, artifact, clean_version, artifact, clean_version, subparts[1]));
        }
        (None, "jar")
    } else {
        return None;
    };

    let filename = match classifier {
        Some(cls) => format!("{}-{}-{}.{}", artifact, version, cls, ext),
        None => format!("{}-{}.{}", artifact, version, ext),
    };

    Some(format!("{}/{}/{}/{}", group, artifact, version, filename))
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Library {
    pub name: String,
    pub downloads: Option<LibraryDownloads>,
    pub rules: Option<Vec<Rule>>,
    pub natives: Option<HashMap<String, String>>,
    pub url: Option<String>,
    pub sha1: Option<String>,
    pub size: Option<u64>,
}

impl Library {
    pub fn get_artifact(&self) -> Option<Artifact> {
        if let Some(ref downloads) = self.downloads
            && let Some(ref art) = downloads.artifact {
                return Some(art.clone());
            }

        if let Some(path) = maven_to_path(&self.name) {
            let sha1 = self.sha1.clone().unwrap_or_default();
            let size = self.size.unwrap_or(0);
            let base_url = self.url.as_deref().unwrap_or("https://repo1.maven.org/maven2/");
            let url = if base_url.ends_with('/') {
                format!("{}{}", base_url, path)
            } else {
                format!("{}/{}", base_url, path)
            };

            Some(Artifact {
                path,
                sha1,
                size,
                url,
            })
        } else {
            None
        }
    }
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
pub struct ClientArtifact {
    pub sha1: String,
    pub size: u64,
    pub url: String,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct DownloadsRef {
    pub client: ClientArtifact,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct JavaVersion {
    pub component: String,
    pub majorVersion: u32,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct VersionDetails {
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub mainClass: Option<String>,
    pub arguments: Option<Arguments>,
    pub minecraftArguments: Option<String>,
    pub libraries: Vec<Library>,
    #[serde(rename = "mavenFiles")]
    pub maven_files: Option<Vec<Library>>,
    pub assetIndex: Option<AssetIndexRef>,
    pub downloads: Option<DownloadsRef>,
    pub javaVersion: Option<JavaVersion>,
    pub inheritsFrom: Option<String>,
    pub uid: Option<String>,
    pub version: Option<String>,
}

impl VersionDetails {
    pub fn id(&self) -> String {
        self.id.clone()
            .or_else(|| {
                if let (Some(uid), Some(ver)) = (&self.uid, &self.version) {
                    if uid.contains("forge") {
                        if uid.contains("neoforged") {
                            Some(format!("neoforge-{}", ver))
                        } else {
                            Some(format!("forge-{}", ver))
                        }
                    } else {
                        Some(ver.clone())
                    }
                } else {
                    self.version.clone()
                }
            })
            .unwrap_or_else(|| "unknown".to_string())
    }
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

#[derive(Clone)]
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

    pub async fn fetch_fabric_loaders(&self, game_version: &str) -> Result<Vec<FabricLoaderResponse>, String> {
        let url = format!("https://meta.fabricmc.net/v2/versions/loader/{}", game_version);
        self.client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Fabric loader versions: {}", e))?
            .json::<Vec<FabricLoaderResponse>>()
            .await
            .map_err(|e| format!("Failed to parse Fabric loader versions: {}", e))
    }

    pub async fn fetch_fabric_profile(&self, game_version: &str, loader_version: &str) -> Result<VersionDetails, String> {
        let url = format!("https://meta.fabricmc.net/v2/versions/loader/{}/{}/profile/json", game_version, loader_version);
        self.client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Fabric profile: {}", e))?
            .json::<VersionDetails>()
            .await
            .map_err(|e| format!("Failed to parse Fabric profile: {}", e))
    }

    pub async fn fetch_forge_versions(&self) -> Result<PrismMetaIndex, String> {
        let url = "https://meta.prismlauncher.org/v1/net.minecraftforge/index.json";
        self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Forge versions: {}", e))?
            .json::<PrismMetaIndex>()
            .await
            .map_err(|e| format!("Failed to parse Forge versions: {}", e))
    }

    pub async fn fetch_neoforge_versions(&self) -> Result<PrismMetaIndex, String> {
        let url = "https://meta.prismlauncher.org/v1/net.neoforged/index.json";
        self.client.get(url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch NeoForge versions: {}", e))?
            .json::<PrismMetaIndex>()
            .await
            .map_err(|e| format!("Failed to parse NeoForge versions: {}", e))
    }

    pub async fn fetch_forge_profile(&self, version: &str) -> Result<VersionDetails, String> {
        let url = format!("https://meta.prismlauncher.org/v1/net.minecraftforge/{}.json", version);
        self.client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Forge profile: {}", e))?
            .json::<VersionDetails>()
            .await
            .map_err(|e| format!("Failed to parse Forge profile: {}", e))
    }

    pub async fn fetch_neoforge_profile(&self, version: &str) -> Result<VersionDetails, String> {
        let url = format!("https://meta.prismlauncher.org/v1/net.neoforged/{}.json", version);
        self.client.get(&url)
            .send()
            .await
            .map_err(|e| format!("Failed to fetch NeoForge profile: {}", e))?
            .json::<VersionDetails>()
            .await
            .map_err(|e| format!("Failed to parse NeoForge profile: {}", e))
    }

    pub async fn search_modpacks(&self, query: &str) -> Result<Vec<ModrinthSearchHit>, String> {
        let url = "https://api.modrinth.com/v2/search";
        self.client.get(url)
            .query(&[("query", query), ("facets", "[[\"project_type:modpack\"]]")])
            .header("User-Agent", "minecli/0.1.0 (contact@minecli.invalid)")
            .send()
            .await
            .map_err(|e| format!("Failed to search Modrinth: {}", e))?
            .json::<ModrinthSearchResponse>()
            .await
            .map(|r| r.hits)
            .map_err(|e| format!("Failed to parse Modrinth search response: {}", e))
    }

    pub async fn fetch_modpack_versions(&self, project_id: &str) -> Result<Vec<ModrinthVersion>, String> {
        let url = format!("https://api.modrinth.com/v2/project/{}/version", project_id);
        self.client.get(&url)
            .header("User-Agent", "minecli/0.1.0 (contact@minecli.invalid)")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch modpack versions: {}", e))?
            .json::<Vec<ModrinthVersion>>()
            .await
            .map_err(|e| format!("Failed to parse modpack versions: {}", e))
    }

    pub async fn search_projects(
        &self,
        query: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
        project_type: &str,
    ) -> Result<Vec<ModrinthSearchHit>, String> {
        let url = "https://api.modrinth.com/v2/search";
        
        let mut facets = vec![vec![format!("project_type:{}", project_type)]];
        if let Some(v) = game_version {
            facets.push(vec![format!("versions:{}", v)]);
        }
        if let Some(l) = loader {
            if project_type == "mod" {
                facets.push(vec![format!("categories:{}", l.to_lowercase())]);
            }
        }

        let facets_json = serde_json::to_string(&facets).map_err(|e| e.to_string())?;

        self.client.get(url)
            .query(&[("query", query), ("facets", &facets_json)])
            .header("User-Agent", "minecli/0.1.0 (contact@minecli.invalid)")
            .send()
            .await
            .map_err(|e| format!("Failed to search Modrinth: {}", e))?
            .json::<ModrinthSearchResponse>()
            .await
            .map(|r| r.hits)
            .map_err(|e| format!("Failed to parse Modrinth search response: {}", e))
    }

    pub async fn search_mods(
        &self,
        query: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<ModrinthSearchHit>, String> {
        self.search_projects(query, game_version, loader, "mod").await
    }

    pub async fn fetch_project(&self, project_id: &str) -> Result<ModrinthProject, String> {
        let url = format!("https://api.modrinth.com/v2/project/{}", project_id);
        self.client.get(&url)
            .header("User-Agent", "minecli/0.1.0 (contact@minecli.invalid)")
            .send()
            .await
            .map_err(|e| format!("Failed to fetch Modrinth project: {}", e))?
            .json::<ModrinthProject>()
            .await
            .map_err(|e| format!("Failed to parse Modrinth project: {}", e))
    }
}

// --- Fabric response models ---
#[derive(Deserialize, Debug, Clone)]
pub struct FabricLoader {
    pub version: String,
    pub stable: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct FabricLoaderResponse {
    pub loader: FabricLoader,
}

// --- Prism Launcher Meta Response models (Forge/NeoForge) ---
#[derive(Deserialize, Debug, Clone)]
pub struct PrismMetaRequire {
    pub equals: String,
    pub uid: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PrismMetaVersion {
    pub version: String,
    pub recommended: bool,
    pub requires: Vec<PrismMetaRequire>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct PrismMetaIndex {
    pub name: String,
    pub uid: String,
    pub versions: Vec<PrismMetaVersion>,
}

// --- Modrinth search API response models ---
#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthSearchHit {
    #[serde(rename = "project_id")]
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    pub downloads: u64,
    pub author: String,
    #[serde(rename = "latest_version")]
    pub latest_version: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthSearchResponse {
    pub hits: Vec<ModrinthSearchHit>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthVersionFile {
    pub url: String,
    pub filename: String,
    pub primary: bool,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthDependency {
    pub version_id: Option<String>,
    pub project_id: Option<String>,
    pub file_name: Option<String>,
    pub dependency_type: String, // "required", "optional", "incompatible", "embedded"
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthVersion {
    pub id: String,
    pub name: String,
    #[serde(rename = "version_number")]
    pub version_number: String,
    pub files: Vec<ModrinthVersionFile>,
    #[serde(rename = "game_versions")]
    pub game_versions: Vec<String>,
    #[serde(default)]
    pub loaders: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<ModrinthDependency>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ModrinthProject {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_maven_to_path() {
        assert_eq!(
            maven_to_path("net.fabricmc:fabric-loader:0.15.0"),
            Some("net/fabricmc/fabric-loader/0.15.0/fabric-loader-0.15.0.jar".to_string())
        );
        assert_eq!(
            maven_to_path("org.ow2.asm:asm-commons:6.2"),
            Some("org/ow2/asm/asm-commons/6.2/asm-commons-6.2.jar".to_string())
        );
        assert_eq!(
            maven_to_path("net.minecraftforge:forge:1.14.4-28.2.30:launcher"),
            Some("net/minecraftforge/forge/1.14.4-28.2.30/forge-1.14.4-28.2.30-launcher.jar".to_string())
        );
        assert_eq!(
            maven_to_path("invalid_coords"),
            None
        );
    }
}
