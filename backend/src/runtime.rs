use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    ffi::OsString,
    fmt,
    fs::{self as std_fs, File as StdFile, OpenOptions as StdOpenOptions},
    io::{self, Write},
    path::{Component, Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
    time::Duration,
};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::{fs, io::AsyncWriteExt, process::Command, task, time::timeout};
use uuid::Uuid;

pub const RECOMMENDED_JAVA: u32 = 21;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProcessMetrics {
    pub(crate) cpu: u8,
    pub(crate) memory: u64,
}

pub(crate) fn sample_process_metrics(system: &mut System, pid: u32) -> Option<ProcessMetrics> {
    let pid = Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().with_cpu().with_memory(),
    );
    let process = system.process(pid)?;
    Some(ProcessMetrics {
        cpu: process.cpu_usage().clamp(0.0, 100.0).round() as u8,
        memory: process.memory().div_ceil(1024 * 1024),
    })
}
/// 托管运行时只保留 Minecraft 服务端实际常用的三个 Java 世代。
///
/// 不能用“Java 版本大于等于 21 就兼容”替代精确选择：Paper 1.12.x
/// 等旧核心在 Java 8 上最稳定，而现代核心需要 Java 17/21。
const SUPPORTED_JAVA_MAJORS: &[u32] = &[8, 17, RECOMMENDED_JAVA];
const JAVA_PROBE_TIMEOUT: Duration = Duration::from_secs(12);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(45);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const MAX_METADATA_BYTES: usize = 2 * 1024 * 1024;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_EXTRACTED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
static DATA_ROOT: OnceLock<PathBuf> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JavaArchiveKind {
    Zip,
    TarGz,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct JavaPackagePlatform {
    api_os: &'static str,
    api_arch: &'static str,
    archive: JavaArchiveKind,
    label: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct JavaInfo {
    pub java_installed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_major: Option<u32>,
    pub java_compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_executable: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub java_home: Option<String>,
}

impl JavaInfo {
    fn unavailable() -> Self {
        Self {
            java_installed: false,
            java_version: None,
            java_major: None,
            java_compatible: false,
            java_executable: None,
            java_home: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SystemInfo {
    #[serde(flatten)]
    pub java: JavaInfo,
    pub os: String,
    pub arch: String,
    pub data_dir: String,
    pub data_dir_writable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir_free_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_memory_bytes: Option<u64>,
    pub logical_cpu_count: usize,
    pub recommended_java: u32,
    pub java_install_supported: bool,
    pub java_install_hint: String,
    pub cores: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallErrorKind {
    UnsupportedPlatform,
    Network,
    Integrity,
    Archive,
    Filesystem,
    Validation,
}

#[derive(Debug)]
pub struct InstallError {
    pub kind: InstallErrorKind,
    message: String,
}

impl InstallError {
    fn new(kind: InstallErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InstallError {}

#[derive(Debug, Deserialize)]
struct AdoptiumAsset {
    binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    checksum: String,
    link: String,
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct MslJavaResponse {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<MslJavaPackage>,
}

#[derive(Debug, Deserialize)]
struct MslJavaPackage {
    url: String,
    sha256: String,
}

pub fn data_root() -> PathBuf {
    DATA_ROOT
        .get_or_init(|| {
            let configured = nonempty_env_os("SCULK_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("data"));
            absolute_path(&configured)
        })
        .clone()
}

pub fn state_file() -> PathBuf {
    nonempty_env_os("SCULK_STATE_FILE")
        .map(PathBuf::from)
        .map(|path| absolute_path(&path))
        .unwrap_or_else(|| data_root().join("state.json"))
}

pub fn server_directory(id: &str) -> PathBuf {
    data_root().join("servers").join(id)
}

pub fn project_directory(id: &str) -> PathBuf {
    data_root().join("projects").join(id)
}

pub fn managed_java_path(data_root: &Path, major: u32) -> PathBuf {
    data_root
        .join("runtimes")
        .join("java")
        .join(major.to_string())
        .join("bin")
        .join(java_binary_name())
}

pub fn is_supported_major(major: u32) -> bool {
    SUPPORTED_JAVA_MAJORS.contains(&major)
}

fn java_package_platform(os: &str, arch: &str) -> Option<JavaPackagePlatform> {
    match (os, arch) {
        ("windows", "x86_64") => Some(JavaPackagePlatform {
            api_os: "windows",
            api_arch: "x64",
            archive: JavaArchiveKind::Zip,
            label: "Windows x64",
        }),
        ("linux", "x86_64") => Some(JavaPackagePlatform {
            api_os: "linux",
            api_arch: "x64",
            archive: JavaArchiveKind::TarGz,
            label: "Linux x64",
        }),
        ("linux", "aarch64") => Some(JavaPackagePlatform {
            api_os: "linux",
            api_arch: "aarch64",
            archive: JavaArchiveKind::TarGz,
            label: "Linux ARM64",
        }),
        _ => None,
    }
}

pub async fn collect_system_info(data_root: &Path) -> SystemInfo {
    let server_data_dir = absolute_path(&data_root.join("servers"));
    let java = detect_java(data_root).await;
    let data_dir_writable = probe_directory_writable(&server_data_dir).await;
    let data_dir_free_bytes = available_space(server_data_dir.clone()).await;
    let total_memory_bytes = total_memory_bytes().await;
    let java_install_supported =
        java_package_platform(std::env::consts::OS, std::env::consts::ARCH).is_some();

    SystemInfo {
        java,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        data_dir: server_data_dir.to_string_lossy().into_owned(),
        data_dir_writable,
        data_dir_free_bytes,
        total_memory_bytes,
        logical_cpu_count: std::thread::available_parallelism()
            .map(usize::from)
            .unwrap_or(1),
        recommended_java: RECOMMENDED_JAVA,
        java_install_supported,
        java_install_hint: if java_install_supported {
            format!("可以安装托管 Java {RECOMMENDED_JAVA}")
        } else {
            format!("请使用系统包管理器安装 Java {RECOMMENDED_JAVA}，或设置 SCULK_JAVA_BIN")
        },
        cores: vec![
            "Paper".into(),
            "Purpur".into(),
            "Fabric".into(),
            "Velocity".into(),
        ],
    }
}

pub async fn detect_java(data_root: &Path) -> JavaInfo {
    detect_java_for_major(data_root, RECOMMENDED_JAVA).await
}

/// 按服务端需要的 Java 世代检测运行时。
///
/// 返回值仍会保留“找到但不兼容”的 Java 信息，便于 UI 展示；调用方应
/// 检查 `java_compatible`，而不是仅检查 `java_installed`。
pub async fn detect_java_for_major(data_root: &Path, required_major: u32) -> JavaInfo {
    let explicit = nonempty_env_os("SCULK_JAVA_BIN").map(PathBuf::from);
    let managed = is_supported_major(required_major)
        .then(|| managed_java_path(data_root, required_major))
        .filter(|path| path.is_file());
    let java_home = nonempty_env_os("JAVA_HOME")
        .map(PathBuf::from)
        .map(|home| home.join("bin").join(java_binary_name()));
    let path_java = std::env::var_os("PATH").and_then(find_java_on_path);

    let candidates = candidate_paths(explicit, managed, java_home, path_java);
    let mut first_found = None;
    for candidate in candidates {
        let Some(executable) = resolve_executable(candidate) else {
            continue;
        };
        if let Some(info) = inspect_java_executable(&executable).await {
            if first_found.is_none() {
                first_found = Some(info.clone());
            }
            if info.java_major == Some(required_major) {
                return java_info_with_compatibility(info, required_major);
            }
        }
    }
    first_found
        .map(|info| java_info_with_compatibility(info, required_major))
        .unwrap_or_else(JavaInfo::unavailable)
}

fn java_info_with_compatibility(mut info: JavaInfo, required_major: u32) -> JavaInfo {
    info.java_compatible = info.java_major == Some(required_major);
    info
}

/// 根据 Minecraft 版本选择精确的 Java 世代。
///
/// 版本未知时使用现代默认值 21；这不会把旧版本误判成可运行，已知旧
/// 版本则会触发对应的托管运行时安装。
pub fn required_java_major(minecraft_version: &str) -> u32 {
    let mut parts = minecraft_version
        .trim()
        .split('.')
        .filter_map(|part| part.parse::<u32>().ok());
    let Some(first) = parts.next() else {
        return RECOMMENDED_JAVA;
    };
    if first != 1 {
        return RECOMMENDED_JAVA;
    }
    let minor = parts.next().unwrap_or_default();
    match minor {
        0..=16 => 8,
        17..=20 => {
            if minor == 20 && parts.next().unwrap_or_default() >= 5 {
                RECOMMENDED_JAVA
            } else {
                17
            }
        }
        _ => RECOMMENDED_JAVA,
    }
}

pub async fn install_java(data_root: &Path, major: u32) -> Result<JavaInfo, InstallError> {
    if !is_supported_major(major) {
        return Err(InstallError::new(
            InstallErrorKind::Validation,
            format!("暂不支持安装 Java {major}，当前支持 Java 8、17、21"),
        ));
    }
    let platform =
        java_package_platform(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
            InstallError::new(
                InstallErrorKind::UnsupportedPlatform,
                format!(
                    "托管 Java 安装暂不支持 {} {}；当前支持 Windows x64、Linux x64 和 Linux ARM64",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ),
            )
        })?;

    let target = managed_runtime_home(data_root, major);
    let target_executable = target.join("bin").join(java_binary_name());
    if let Some(info) = inspect_java_executable(&target_executable).await
        && info.java_major == Some(major)
    {
        return Ok(java_info_with_compatibility(info, major));
    }

    let runtime_parent = target
        .parent()
        .ok_or_else(|| InstallError::new(InstallErrorKind::Filesystem, "托管运行时目录无效"))?;
    fs::create_dir_all(runtime_parent).await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法创建托管运行时目录：{error}"),
        )
    })?;

    let nonce = Uuid::new_v4().simple().to_string();
    let archive = runtime_parent.join(format!(
        ".java-{major}-{nonce}.{}.part",
        match platform.archive {
            JavaArchiveKind::Zip => "zip",
            JavaArchiveKind::TarGz => "tar.gz",
        }
    ));
    let staging = runtime_parent.join(format!(".java-{major}-{nonce}.extracting"));
    let ready = runtime_parent.join(format!(".java-{major}-{nonce}.ready"));
    let backup = runtime_parent.join(format!(".java-{major}-{nonce}.backup"));

    let result = install_java_inner(
        major, &target, &archive, &staging, &ready, &backup, platform,
    )
    .await;

    remove_file_if_present(&archive).await;
    remove_dir_if_present(&staging).await;
    remove_dir_if_present(&ready).await;
    if target.exists() {
        remove_dir_if_present(&backup).await;
    }
    result.map(|info| java_info_with_compatibility(info, major))
}

async fn install_java_inner(
    major: u32,
    target: &Path,
    archive: &Path,
    staging: &Path,
    ready: &Path,
    backup: &Path,
    platform: JavaPackagePlatform,
) -> Result<JavaInfo, InstallError> {
    let client = reqwest::Client::builder()
        .user_agent("Sculk-Catalyst/3 managed-java-installer")
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .timeout(REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.url().scheme() == "https" {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| InstallError::new(InstallErrorKind::Network, "无法初始化 Java 下载客户端"))?;

    let package = fetch_package_metadata(&client, major, platform).await?;
    download_and_verify(&client, &package, archive).await?;
    fs::create_dir(staging).await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法创建 Java 解压目录：{error}"),
        )
    })?;
    extract_archive(archive.to_owned(), staging.to_owned(), platform.archive).await?;
    remove_file_if_present(archive).await;

    let extracted_home = find_java_home(staging).ok_or_else(|| {
        InstallError::new(
            InstallErrorKind::Archive,
            format!("Java 压缩包中未找到 bin/{}", java_binary_name()),
        )
    })?;
    let extracted_executable = extracted_home.join("bin").join(java_binary_name());
    let extracted_info = inspect_java_executable(&extracted_executable)
        .await
        .ok_or_else(|| {
            InstallError::new(
                InstallErrorKind::Validation,
                "下载的 Java 无法运行或版本输出无法识别",
            )
        })?;
    if extracted_info.java_major != Some(major) {
        return Err(InstallError::new(
            InstallErrorKind::Validation,
            format!("下载的 Java 版本不是请求的 Java {major}"),
        ));
    }

    fs::rename(&extracted_home, ready).await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法准备 Java 安装目录：{error}"),
        )
    })?;
    if staging.exists() {
        remove_dir_if_present(staging).await;
    }

    let ready_executable = ready.join("bin").join(java_binary_name());
    let ready_info = inspect_java_executable(&ready_executable)
        .await
        .ok_or_else(|| {
            InstallError::new(
                InstallErrorKind::Validation,
                "Java 安装目录准备完成，但运行验证失败",
            )
        })?;
    if ready_info.java_major != Some(major) {
        return Err(InstallError::new(
            InstallErrorKind::Validation,
            "Java 安装后的版本校验失败",
        ));
    }
    atomic_replace_directory(ready, target, backup).await?;
    Ok(java_info_at_home(ready_info, target))
}

async fn fetch_package_metadata(
    client: &reqwest::Client,
    major: u32,
    platform: JavaPackagePlatform,
) -> Result<AdoptiumPackage, InstallError> {
    match fetch_msl_package_metadata(client, major, platform).await {
        Ok(package) => Ok(package),
        Err(msl_error) => match fetch_adoptium_package_metadata(client, major, platform).await {
            Ok(package) => Ok(package),
            Err(adoptium_error) => Err(InstallError::new(
                adoptium_error.kind,
                format!(
                    "MSL 镜像未提供 Java {major} {}：{}；Eclipse Adoptium 回退也失败：{}",
                    platform.label, msl_error.message, adoptium_error.message
                ),
            )),
        },
    }
}

fn msl_java_metadata_url(major: u32, platform: JavaPackagePlatform) -> String {
    format!(
        "https://api.mslmc.cn/v4/download/jdk/{major}?os={}&arch={}",
        platform.api_os, platform.api_arch
    )
}

async fn fetch_msl_package_metadata(
    client: &reqwest::Client,
    major: u32,
    platform: JavaPackagePlatform,
) -> Result<AdoptiumPackage, InstallError> {
    let response = client
        .get(msl_java_metadata_url(major, platform))
        .header("User-Agent", "MSL API Test")
        .send()
        .await
        .map_err(|error| InstallError::new(InstallErrorKind::Network, error.to_string()))?;
    if !response.status().is_success() {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            format!("MSL Java API HTTP {}", response.status().as_u16()),
        ));
    }
    let payload: MslJavaResponse = response.json().await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Network,
            format!("MSL Java API 响应格式无效：{error}"),
        )
    })?;
    if payload.code != 200 {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            if payload.message.trim().is_empty() {
                "MSL Java API 没有可用版本".into()
            } else {
                payload.message
            },
        ));
    }
    let package = payload
        .data
        .ok_or_else(|| InstallError::new(InstallErrorKind::Network, "MSL Java API 缺少下载信息"))?;
    let package = AdoptiumPackage {
        checksum: package.sha256,
        link: package.url,
        size: None,
    };
    validate_package_metadata(&package)?;
    Ok(package)
}

async fn fetch_adoptium_package_metadata(
    client: &reqwest::Client,
    major: u32,
    platform: JavaPackagePlatform,
) -> Result<AdoptiumPackage, InstallError> {
    let metadata_url = format!(
        "https://api.adoptium.net/v3/assets/latest/{major}/hotspot?architecture={}&heap_size=normal&image_type=jre&jvm_impl=hotspot&os={}&project=jdk&vendor=eclipse",
        platform.api_arch, platform.api_os
    );
    let response = client.get(metadata_url).send().await.map_err(|_| {
        InstallError::new(
            InstallErrorKind::Network,
            "无法连接 Eclipse Adoptium 获取 Java 下载信息",
        )
    })?;
    if !response.status().is_success() {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            format!(
                "Eclipse Adoptium 下载信息请求失败（HTTP {}）",
                response.status().as_u16()
            ),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_METADATA_BYTES as u64)
    {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            "Eclipse Adoptium 返回的下载信息过大",
        ));
    }
    let bytes = response.bytes().await.map_err(|_| {
        InstallError::new(
            InstallErrorKind::Network,
            "读取 Eclipse Adoptium 下载信息失败",
        )
    })?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            "Eclipse Adoptium 返回的下载信息过大",
        ));
    }
    let assets: Vec<AdoptiumAsset> = serde_json::from_slice(&bytes).map_err(|_| {
        InstallError::new(
            InstallErrorKind::Network,
            "Eclipse Adoptium 返回的下载信息格式无效",
        )
    })?;
    let package = assets
        .into_iter()
        .next()
        .map(|asset| asset.binary.package)
        .ok_or_else(|| {
            InstallError::new(
                InstallErrorKind::Network,
                format!(
                    "Eclipse Adoptium 暂无可用的 Java {major} {} 包",
                    platform.label
                ),
            )
        })?;
    validate_package_metadata(&package)?;
    Ok(package)
}

fn validate_package_metadata(package: &AdoptiumPackage) -> Result<(), InstallError> {
    if package.checksum.len() != 64
        || !package
            .checksum
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(InstallError::new(
            InstallErrorKind::Integrity,
            "Eclipse Adoptium 未提供有效的 SHA-256 校验值",
        ));
    }
    let url = reqwest::Url::parse(&package.link)
        .map_err(|_| InstallError::new(InstallErrorKind::Network, "Java 下载地址格式无效"))?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            "Java 下载地址不是安全的 HTTPS 地址",
        ));
    }
    if package.size.is_some_and(|size| size > MAX_ARCHIVE_BYTES) {
        return Err(InstallError::new(
            InstallErrorKind::Integrity,
            "Java 下载包大小超过安全限制",
        ));
    }
    Ok(())
}

async fn download_and_verify(
    client: &reqwest::Client,
    package: &AdoptiumPackage,
    destination: &Path,
) -> Result<(), InstallError> {
    let response = client
        .get(&package.link)
        .send()
        .await
        .map_err(|_| InstallError::new(InstallErrorKind::Network, "下载 Java 运行时失败"))?;
    if !response.status().is_success() {
        return Err(InstallError::new(
            InstallErrorKind::Network,
            format!(
                "下载 Java 运行时失败（HTTP {}）",
                response.status().as_u16()
            ),
        ));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_ARCHIVE_BYTES)
    {
        return Err(InstallError::new(
            InstallErrorKind::Integrity,
            "Java 下载包大小超过安全限制",
        ));
    }

    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(destination)
        .await
        .map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("无法创建 Java 临时下载文件：{error}"),
            )
        })?;
    let mut response = response;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| InstallError::new(InstallErrorKind::Network, "读取 Java 下载数据失败"))?
    {
        downloaded = downloaded
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| InstallError::new(InstallErrorKind::Integrity, "Java 下载包大小无效"))?;
        if downloaded > MAX_ARCHIVE_BYTES
            || package.size.is_some_and(|expected| downloaded > expected)
        {
            return Err(InstallError::new(
                InstallErrorKind::Integrity,
                "Java 下载包大小与元数据不一致",
            ));
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await.map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("写入 Java 临时下载文件失败：{error}"),
            )
        })?;
    }
    if package.size.is_some_and(|expected| downloaded != expected) {
        return Err(InstallError::new(
            InstallErrorKind::Integrity,
            "Java 下载包大小与元数据不一致",
        ));
    }
    file.flush().await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("刷新 Java 临时下载文件失败：{error}"),
        )
    })?;
    file.sync_all().await.map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("同步 Java 临时下载文件失败：{error}"),
        )
    })?;
    drop(file);

    let actual_checksum = format!("{:x}", hasher.finalize());
    if !actual_checksum.eq_ignore_ascii_case(&package.checksum) {
        return Err(InstallError::new(
            InstallErrorKind::Integrity,
            "Java 下载包 SHA-256 校验失败",
        ));
    }
    Ok(())
}

async fn extract_archive(
    archive: PathBuf,
    destination: PathBuf,
    kind: JavaArchiveKind,
) -> Result<(), InstallError> {
    task::spawn_blocking(move || match kind {
        JavaArchiveKind::Zip => extract_zip_archive_sync(&archive, &destination),
        JavaArchiveKind::TarGz => extract_tar_gz_archive_sync(&archive, &destination),
    })
    .await
    .map_err(|_| InstallError::new(InstallErrorKind::Archive, "Java 解压任务异常终止"))?
}

fn extract_zip_archive_sync(archive: &Path, destination: &Path) -> Result<(), InstallError> {
    let file = StdFile::open(archive).map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法读取 Java 下载包：{error}"),
        )
    })?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| {
        InstallError::new(InstallErrorKind::Archive, "Java 下载包不是有效的 ZIP 文件")
    })?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(InstallError::new(
            InstallErrorKind::Archive,
            "Java 下载包文件数量超过安全限制",
        ));
    }

    let mut total_size = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| {
            InstallError::new(InstallErrorKind::Archive, "无法读取 Java 下载包条目")
        })?;
        total_size = total_size
            .checked_add(entry.size())
            .ok_or_else(|| InstallError::new(InstallErrorKind::Archive, "Java 解压大小无效"))?;
        if total_size > MAX_EXTRACTED_BYTES {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 解压大小超过安全限制",
            ));
        }
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 下载包包含不允许的符号链接",
            ));
        }
        let relative = safe_archive_path(entry.name()).ok_or_else(|| {
            InstallError::new(InstallErrorKind::Archive, "Java 下载包包含不安全的文件路径")
        })?;
        let output = destination.join(relative);
        if entry.is_dir() {
            std_fs::create_dir_all(&output).map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压目录：{error}"),
                )
            })?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std_fs::create_dir_all(parent).map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压目录：{error}"),
                )
            })?;
        }
        let mut output_file = StdOpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压文件：{error}"),
                )
            })?;
        let written = io::copy(&mut entry, &mut output_file).map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("写入 Java 解压文件失败：{error}"),
            )
        })?;
        if written != entry.size() {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 解压文件大小与压缩包目录不一致",
            ));
        }
        output_file.flush().map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("刷新 Java 解压文件失败：{error}"),
            )
        })?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn extract_tar_gz_archive_sync(archive: &Path, destination: &Path) -> Result<(), InstallError> {
    use flate2::read::GzDecoder;
    use std::os::unix::fs::PermissionsExt;

    let file = StdFile::open(archive).map_err(|error| {
        InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法读取 Java 下载包：{error}"),
        )
    })?;
    let decoder = GzDecoder::new(file);
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(|_| {
        InstallError::new(
            InstallErrorKind::Archive,
            "Java 下载包不是有效的 tar.gz 文件",
        )
    })?;
    let mut entry_count = 0_usize;
    let mut total_size = 0_u64;
    for entry in entries {
        entry_count += 1;
        if entry_count > MAX_ARCHIVE_ENTRIES {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 下载包文件数量超过安全限制",
            ));
        }
        let mut entry = entry.map_err(|_| {
            InstallError::new(InstallErrorKind::Archive, "无法读取 Java 下载包条目")
        })?;
        let kind = entry.header().entry_type();
        if !kind.is_file() && !kind.is_dir() {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 下载包包含不允许的链接或特殊文件",
            ));
        }
        let size = entry.size();
        total_size = total_size
            .checked_add(size)
            .ok_or_else(|| InstallError::new(InstallErrorKind::Archive, "Java 解压大小无效"))?;
        if total_size > MAX_EXTRACTED_BYTES {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 解压大小超过安全限制",
            ));
        }
        let raw_path = entry
            .path()
            .map_err(|_| InstallError::new(InstallErrorKind::Archive, "Java 下载包条目路径无效"))?;
        let raw_path = raw_path.to_str().ok_or_else(|| {
            InstallError::new(InstallErrorKind::Archive, "Java 下载包条目路径不是 UTF-8")
        })?;
        let relative = safe_archive_path(raw_path).ok_or_else(|| {
            InstallError::new(InstallErrorKind::Archive, "Java 下载包包含不安全的文件路径")
        })?;
        let output = destination.join(relative);
        if kind.is_dir() {
            std_fs::create_dir_all(&output).map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压目录：{error}"),
                )
            })?;
            continue;
        }
        if let Some(parent) = output.parent() {
            std_fs::create_dir_all(parent).map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压目录：{error}"),
                )
            })?;
        }
        let mut output_file = StdOpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&output)
            .map_err(|error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法创建 Java 解压文件：{error}"),
                )
            })?;
        let written = io::copy(&mut entry, &mut output_file).map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("写入 Java 解压文件失败：{error}"),
            )
        })?;
        if written != size {
            return Err(InstallError::new(
                InstallErrorKind::Archive,
                "Java 解压文件大小与压缩包目录不一致",
            ));
        }
        output_file.flush().map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("刷新 Java 解压文件失败：{error}"),
            )
        })?;
        let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
        std_fs::set_permissions(&output, std_fs::Permissions::from_mode(mode)).map_err(
            |error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!("无法设置 Java 文件权限：{error}"),
                )
            },
        )?;
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn extract_tar_gz_archive_sync(_archive: &Path, _destination: &Path) -> Result<(), InstallError> {
    Err(InstallError::new(
        InstallErrorKind::UnsupportedPlatform,
        "当前平台不支持解压 Linux Java tar.gz 包",
    ))
}

fn safe_archive_path(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('\0') {
        return None;
    }
    let normalized = name.replace('\\', "/");
    let path = Path::new(&normalized);
    if path.is_absolute() {
        return None;
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str()?;
                if value.is_empty() || value.contains(':') {
                    return None;
                }
                safe.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!safe.as_os_str().is_empty()).then_some(safe)
}

async fn atomic_replace_directory(
    ready: &Path,
    target: &Path,
    backup: &Path,
) -> Result<(), InstallError> {
    let had_previous = target.exists();
    if had_previous {
        fs::rename(target, backup).await.map_err(|error| {
            InstallError::new(
                InstallErrorKind::Filesystem,
                format!("无法备份现有 Java 安装：{error}"),
            )
        })?;
    }

    if let Err(error) = fs::rename(ready, target).await {
        if had_previous {
            fs::rename(backup, target).await.map_err(|restore_error| {
                InstallError::new(
                    InstallErrorKind::Filesystem,
                    format!(
                        "Java 安装切换失败，且无法恢复原安装：{error}；恢复错误：{restore_error}"
                    ),
                )
            })?;
        }
        return Err(InstallError::new(
            InstallErrorKind::Filesystem,
            format!("无法切换到新的 Java 安装：{error}"),
        ));
    }

    if had_previous {
        remove_dir_if_present(backup).await;
    }
    Ok(())
}

async fn inspect_java_executable(executable: &Path) -> Option<JavaInfo> {
    if !executable.is_file() {
        return None;
    }
    let output = timeout(
        JAVA_PROBE_TIMEOUT,
        Command::new(executable)
            .arg("-version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.stdout.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(&output.stdout));
    }
    let (version, major) = parse_java_version(&text)?;
    let executable = std_fs::canonicalize(executable).unwrap_or_else(|_| absolute_path(executable));
    let java_home = executable
        .parent()
        .and_then(Path::parent)
        .map(|path| path.to_string_lossy().into_owned());
    Some(JavaInfo {
        java_installed: true,
        java_version: Some(version),
        java_major: Some(major),
        java_compatible: major >= RECOMMENDED_JAVA,
        java_executable: Some(executable.to_string_lossy().into_owned()),
        java_home,
    })
}

fn parse_java_version(output: &str) -> Option<(String, u32)> {
    for line in output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let lower = line.to_ascii_lowercase();
        let version = if let Some(position) = lower.find("version") {
            first_version_token(&line[position + "version".len()..])
        } else if lower.starts_with("openjdk ") || lower.starts_with("java ") {
            first_version_token(line.split_once(char::is_whitespace)?.1)
        } else {
            None
        };
        if let Some(version) = version
            && let Some(major) = parse_java_major(&version)
        {
            return Some((version, major));
        }
    }
    None
}

fn first_version_token(value: &str) -> Option<String> {
    let value = value.trim_start();
    let token = if let Some(value) = value.strip_prefix('"') {
        value.split_once('"')?.0
    } else {
        value.split_whitespace().next()?
    };
    let token = token.trim_matches(|character: char| character == '\'' || character == '"');
    token
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_digit())
        .then(|| token.to_string())
}

fn parse_java_major(version: &str) -> Option<u32> {
    let mut components = version.split(['.', '-', '+', '_']);
    let first = components.next()?.parse::<u32>().ok()?;
    if first == 1 {
        components.next()?.parse().ok()
    } else {
        Some(first)
    }
}

fn candidate_paths(
    explicit: Option<PathBuf>,
    managed: Option<PathBuf>,
    java_home: Option<PathBuf>,
    path_java: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for candidate in [explicit, managed, java_home, path_java]
        .into_iter()
        .flatten()
    {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn resolve_executable(candidate: PathBuf) -> Option<PathBuf> {
    if candidate.components().count() == 1 {
        let local = absolute_path(&candidate);
        if local.is_file() {
            return Some(local);
        }
        return std::env::var_os("PATH")
            .and_then(|path| find_named_executable_on_path(path, &candidate));
    }
    let candidate = absolute_path(&candidate);
    candidate.is_file().then_some(candidate)
}

fn find_named_executable_on_path(path: OsString, name: &Path) -> Option<PathBuf> {
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|executable| executable.is_file())
        .map(|executable| absolute_path(&executable))
}

fn find_java_on_path(path: OsString) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["java.exe", "java"]
    } else {
        &["java"]
    };
    for directory in std::env::split_paths(&path) {
        for name in names {
            let executable = directory.join(name);
            if executable.is_file() {
                return Some(absolute_path(&executable));
            }
        }
    }
    None
}

fn find_java_home(root: &Path) -> Option<PathBuf> {
    let mut directories = VecDeque::from([(root.to_owned(), 0_u8)]);
    while let Some((directory, depth)) = directories.pop_front() {
        if directory.join("bin").join(java_binary_name()).is_file() {
            return Some(directory);
        }
        if depth >= 3 {
            continue;
        }
        let mut children = std_fs::read_dir(&directory)
            .ok()?
            .filter_map(Result::ok)
            .filter_map(|entry| {
                entry
                    .file_type()
                    .ok()
                    .filter(|kind| kind.is_dir() && !kind.is_symlink())
                    .map(|_| entry.path())
            })
            .collect::<Vec<_>>();
        children.sort();
        directories.extend(children.into_iter().map(|child| (child, depth + 1)));
    }
    None
}

fn managed_runtime_home(data_root: &Path, major: u32) -> PathBuf {
    data_root
        .join("runtimes")
        .join("java")
        .join(major.to_string())
}

fn java_binary_name() -> &'static str {
    if cfg!(windows) { "java.exe" } else { "java" }
}

fn nonempty_env_os(name: &str) -> Option<OsString> {
    std::env::var_os(name).filter(|value| !value.is_empty())
}

fn absolute_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .unwrap_or_else(|_| path.to_owned())
    }
}

fn java_info_at_home(mut info: JavaInfo, home: &Path) -> JavaInfo {
    let home = std_fs::canonicalize(home).unwrap_or_else(|_| absolute_path(home));
    info.java_home = Some(home.to_string_lossy().into_owned());
    info.java_executable = Some(
        home.join("bin")
            .join(java_binary_name())
            .to_string_lossy()
            .into_owned(),
    );
    info
}

async fn probe_directory_writable(directory: &Path) -> bool {
    if fs::create_dir_all(directory).await.is_err() {
        return false;
    }
    let probe = directory.join(format!(".sculk-write-probe-{}", Uuid::new_v4().simple()));
    let Ok(mut file) = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .await
    else {
        return false;
    };
    let result = async {
        file.write_all(b"sculk-write-probe").await?;
        file.flush().await?;
        file.sync_all().await
    }
    .await;
    let removed = fs::remove_file(&probe).await;
    result.is_ok() && removed.is_ok()
}

async fn available_space(directory: PathBuf) -> Option<u64> {
    task::spawn_blocking(move || fs2::available_space(directory).ok())
        .await
        .ok()
        .flatten()
}

pub(crate) async fn total_memory_bytes() -> Option<u64> {
    task::spawn_blocking(|| {
        let mut system = System::new();
        system.refresh_memory();
        let total = system.total_memory();
        (total > 0).then_some(total)
    })
    .await
    .ok()
    .flatten()
}

/// 给新手开服流程提供保守的初始堆大小；后续仍可依据真实 TPS/GC 调整。
/// 始终为宿主系统保留内存，不会因为用户不了解 JVM 而把整机内存全部分给服务器。
pub(crate) fn recommended_server_memory_gb(
    total_memory_bytes: Option<u64>,
    expected_players: Option<u32>,
    modded: bool,
) -> u8 {
    let players = expected_players.unwrap_or(12);
    let desired = if modded {
        match players {
            0..=10 => 6,
            11..=30 => 8,
            31..=60 => 12,
            _ => 16,
        }
    } else {
        match players {
            0..=12 => 4,
            13..=30 => 6,
            31..=60 => 8,
            _ => 12,
        }
    };
    let Some(total_bytes) = total_memory_bytes else {
        return desired;
    };
    let total_gb = (total_bytes / (1024 * 1024 * 1024)).clamp(1, u8::MAX as u64) as u8;
    let reserve = match total_gb {
        0..=4 => 1,
        5..=8 => 2,
        9..=16 => 3,
        _ => 4,
    };
    desired
        .min(total_gb.saturating_sub(reserve).max(2))
        .clamp(2, 64)
}

async fn remove_file_if_present(path: &Path) {
    match fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

async fn remove_dir_if_present(path: &Path) {
    match fs::remove_dir_all(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_common_java_version_formats() {
        let cases = [
            (r#"openjdk version "21.0.7" 2025-04-15 LTS"#, "21.0.7", 21),
            (r#"java version "1.8.0_402""#, "1.8.0_402", 8),
            ("openjdk 17.0.12 2024-07-16", "17.0.12", 17),
            (r#"openjdk version "21-ea" 2023-09-19"#, "21-ea", 21),
            (r#"java version "22.0.1+8""#, "22.0.1+8", 22),
        ];
        for (output, version, major) in cases {
            assert_eq!(
                parse_java_version(output),
                Some((version.to_string(), major)),
                "failed to parse {output}"
            );
        }
        assert_eq!(parse_java_version("not a java version"), None);
    }

    #[test]
    fn java_candidate_priority_is_explicit_managed_home_then_path() {
        let candidates = candidate_paths(
            Some(PathBuf::from("explicit/java.exe")),
            Some(PathBuf::from("managed/bin/java.exe")),
            Some(PathBuf::from("home/bin/java.exe")),
            Some(PathBuf::from("path/java.exe")),
        );
        assert_eq!(
            candidates,
            [
                PathBuf::from("explicit/java.exe"),
                PathBuf::from("managed/bin/java.exe"),
                PathBuf::from("home/bin/java.exe"),
                PathBuf::from("path/java.exe"),
            ]
        );
    }

    #[test]
    fn managed_java_path_uses_stable_versioned_layout() {
        assert_eq!(
            managed_java_path(Path::new("data"), 21),
            Path::new("data")
                .join("runtimes")
                .join("java")
                .join("21")
                .join("bin")
                .join(java_binary_name())
        );
    }

    #[test]
    fn minecraft_versions_select_the_matching_java_generation() {
        assert_eq!(required_java_major("1.12.2"), 8);
        assert_eq!(required_java_major("1.16.5"), 8);
        assert_eq!(required_java_major("1.17.1"), 17);
        assert_eq!(required_java_major("1.20.4"), 17);
        assert_eq!(required_java_major("1.20.5"), 21);
        assert_eq!(required_java_major("1.21.4"), 21);
        assert_eq!(required_java_major("latest"), 21);
    }

    #[test]
    fn supported_java_majors_cover_legacy_and_modern_servers() {
        assert!(is_supported_major(8));
        assert!(is_supported_major(17));
        assert!(is_supported_major(21));
        assert!(!is_supported_major(11));
    }

    #[test]
    fn memory_recommendation_preserves_host_capacity_for_novice_defaults() {
        let gib = 1024_u64 * 1024 * 1024;
        assert_eq!(
            recommended_server_memory_gb(Some(8 * gib), Some(10), false),
            4
        );
        assert_eq!(
            recommended_server_memory_gb(Some(4 * gib), Some(10), false),
            3
        );
        assert_eq!(
            recommended_server_memory_gb(Some(16 * gib), Some(40), false),
            8
        );
        assert_eq!(
            recommended_server_memory_gb(Some(8 * gib), Some(10), true),
            6
        );
        assert_eq!(recommended_server_memory_gb(None, None, false), 4);
    }

    #[test]
    fn managed_java_packages_cover_supported_windows_and_linux_architectures() {
        let windows = java_package_platform("windows", "x86_64").unwrap();
        assert_eq!(windows.api_os, "windows");
        assert_eq!(windows.api_arch, "x64");
        assert_eq!(windows.archive, JavaArchiveKind::Zip);

        let linux_x64 = java_package_platform("linux", "x86_64").unwrap();
        assert_eq!(linux_x64.api_os, "linux");
        assert_eq!(linux_x64.api_arch, "x64");
        assert_eq!(linux_x64.archive, JavaArchiveKind::TarGz);

        let linux_arm64 = java_package_platform("linux", "aarch64").unwrap();
        assert_eq!(linux_arm64.api_arch, "aarch64");
        assert_eq!(linux_arm64.archive, JavaArchiveKind::TarGz);

        assert!(java_package_platform("windows", "aarch64").is_none());
        assert!(java_package_platform("linux", "arm").is_none());
        assert!(java_package_platform("macos", "aarch64").is_none());
    }

    #[test]
    fn msl_java_metadata_uses_supported_platform_query_shape() {
        let windows = java_package_platform("windows", "x86_64").unwrap();
        assert_eq!(
            msl_java_metadata_url(21, windows),
            "https://api.mslmc.cn/v4/download/jdk/21?os=windows&arch=x64"
        );
        let linux = java_package_platform("linux", "aarch64").unwrap();
        assert_eq!(
            msl_java_metadata_url(17, linux),
            "https://api.mslmc.cn/v4/download/jdk/17?os=linux&arch=aarch64"
        );
    }

    #[test]
    fn discovers_java_home_below_archive_root() {
        let root =
            std::env::temp_dir().join(format!("sculk-java-discovery-{}", Uuid::new_v4().simple()));
        let expected = root.join("jdk-21.0.7+6-jre");
        std_fs::create_dir_all(expected.join("bin")).unwrap();
        StdFile::create(expected.join("bin").join(java_binary_name())).unwrap();

        assert_eq!(find_java_home(&root), Some(expected));
        std_fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_zip_slip_and_windows_alternate_stream_paths() {
        assert_eq!(
            safe_archive_path("jdk/bin/java.exe"),
            Some(PathBuf::from("jdk/bin/java.exe"))
        );
        for unsafe_path in [
            "../outside.exe",
            "jdk/../../outside.exe",
            "/absolute/java.exe",
            r"C:\absolute\java.exe",
            r"..\outside.exe",
            "jdk/bin/java.exe:payload",
        ] {
            assert_eq!(
                safe_archive_path(unsafe_path),
                None,
                "accepted {unsafe_path}"
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn extracts_linux_tar_gz_and_preserves_java_execute_permission() {
        use flate2::{Compression, write::GzEncoder};
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("sculk-java-tar-{}", Uuid::new_v4().simple()));
        let archive_path = root.join("java.tar.gz");
        let destination = root.join("extract");
        std_fs::create_dir_all(&destination).unwrap();
        let archive_file = StdFile::create(&archive_path).unwrap();
        let encoder = GzEncoder::new(archive_file, Compression::default());
        let mut archive = tar::Builder::new(encoder);
        let payload = b"fake-java";
        let mut header = tar::Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        archive
            .append_data(&mut header, "jdk-21/bin/java", &payload[..])
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();

        extract_tar_gz_archive_sync(&archive_path, &destination).unwrap();

        let executable = destination.join("jdk-21/bin/java");
        assert_eq!(std_fs::read(&executable).unwrap(), payload);
        assert_ne!(
            std_fs::metadata(&executable).unwrap().permissions().mode() & 0o111,
            0
        );
        std_fs::remove_dir_all(root).unwrap();
    }
}
