//! 配置模块：负责 config.json 的读取、校验、保存，以及配置损坏时的备份与恢复。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use reqwest::Url;
use serde::{Deserialize, Serialize};

/// 支持的目标协议列表
const SUPPORTED_PROTOCOLS: [&str; 2] = ["openai", "anthropic"];

/// 支持的关窗行为列表
const SUPPORTED_CLOSE_BEHAVIORS: [&str; 2] = ["minimize", "exit"];

/// 默认目标协议：OpenAI 风格
fn default_protocol() -> String {
    "openai".to_string()
}

/// 默认配置版本号
fn default_version() -> u32 {
    1
}

/// 默认关窗行为：最小化到托盘
fn default_close_behavior() -> String {
    "minimize".to_string()
}

/// 单个代理实例的配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProxyInstance {
    /// 实例唯一标识
    pub id: String,
    /// 实例名称（界面展示用）
    pub name: String,
    /// 是否启用
    pub enabled: bool,
    /// 转发目标地址（http/https，可包含路径前缀）
    pub target: String,
    /// 本地监听端口
    pub http_port: u16,
    /// 目标协议：openai（默认）/ anthropic
    #[serde(default = "default_protocol")]
    pub protocol: String,
    /// 推理强度（可选）：配置后按协议向请求体注入默认值
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    /// 附加的静态请求头（与透传头同名时以此为准）
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// 配置版本号
    #[serde(default = "default_version")]
    pub version: u32,
    /// 关闭主窗口时的行为：minimize（最小化到托盘）/ exit（退出应用）
    #[serde(default = "default_close_behavior")]
    pub close_behavior: String,
    /// 代理实例列表
    #[serde(default)]
    pub proxies: Vec<ProxyInstance>,
}

impl Default for AppConfig {
    /// 生成默认空配置（版本 1、最小化到托盘、无代理实例）
    fn default() -> Self {
        Self {
            version: default_version(),
            close_behavior: default_close_behavior(),
            proxies: Vec::new(),
        }
    }
}

/// 计算配置文件的备份路径（同目录下的 `<文件名>.bak`，如 config.json.bak）
fn backup_path(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "config.json".into());
    file_name.push(".bak");
    path.with_file_name(file_name)
}

/// 读取并解析配置文件。
///
/// - 文件不存在：返回默认空配置，警告为 `None`
/// - 解析失败：把原文件备份为同目录下的 `config.json.bak`，返回默认空配置 + 中文警告
/// - 其它 IO 错误：直接返回错误
pub fn load(path: &Path) -> Result<(AppConfig, Option<String>), std::io::Error> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok((AppConfig::default(), None)),
        Err(err) => return Err(err),
    };

    match serde_json::from_str::<AppConfig>(&text) {
        Ok(config) => Ok((config, None)),
        Err(err) => {
            // 配置已损坏：先备份原文件，避免用户数据直接丢失
            let backup = backup_path(path);
            std::fs::rename(path, &backup)?;
            let backup_name = backup
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "config.json.bak".to_string());
            let warning = format!("配置已损坏（{err}），已备份为 {backup_name} 并以空配置启动");
            Ok((AppConfig::default(), Some(warning)))
        }
    }
}

/// 保存配置：先校验，再写入磁盘（自动创建父目录，JSON 美化输出）。
///
/// 校验失败或写盘失败时返回中文错误信息。
pub fn save(path: &Path, config: &AppConfig) -> Result<(), String> {
    validate(config)?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| format!("创建配置目录失败: {err}"))?;
        }
    }

    let text =
        serde_json::to_string_pretty(config).map_err(|err| format!("序列化配置失败: {err}"))?;
    std::fs::write(path, text).map_err(|err| format!("写入配置文件失败: {err}"))?;
    Ok(())
}

/// 校验配置的合法性，返回全部中文错误信息（以「；」分隔）。
///
/// 校验规则：
/// - 每个实例的 target 必须是合法的 http/https URL
/// - 每个实例的 httpPort 不能为 0，且实例之间不能重复
/// - 每个实例的 protocol 只能是 openai 或 anthropic
/// - 每个实例的 id 不能为空，且实例之间不能重复
/// - closeBehavior 只能是 minimize 或 exit
pub fn validate(config: &AppConfig) -> Result<(), String> {
    let mut errors: Vec<String> = Vec::new();

    if !SUPPORTED_CLOSE_BEHAVIORS.contains(&config.close_behavior.as_str()) {
        errors.push(format!(
            "closeBehavior 取值非法（{}），仅支持 minimize 或 exit",
            config.close_behavior
        ));
    }

    let mut ids: HashSet<&str> = HashSet::new();
    let mut ports: HashMap<u16, String> = HashMap::new();

    for (index, instance) in config.proxies.iter().enumerate() {
        // 错误信息优先使用实例名称，缺失时退化为序号与 id
        let label = if instance.name.trim().is_empty() {
            if instance.id.trim().is_empty() {
                format!("第 {} 个实例", index + 1)
            } else {
                format!("实例「{}」", instance.id)
            }
        } else {
            format!("实例「{}」", instance.name)
        };

        // id 非空且唯一
        if instance.id.trim().is_empty() {
            errors.push(format!("{label} 的 id 不能为空"));
        } else if !ids.insert(instance.id.as_str()) {
            errors.push(format!("{label} 的 id「{}」重复", instance.id));
        }

        // target 必须是合法的 http/https URL
        match Url::parse(&instance.target) {
            Ok(url) => {
                if url.scheme() != "http" && url.scheme() != "https" {
                    errors.push(format!(
                        "{label} 的 target 协议不受支持（{}），仅支持 http 或 https",
                        url.scheme()
                    ));
                } else if url.host_str().is_none() {
                    errors.push(format!(
                        "{label} 的 target 缺少主机名（{}）",
                        instance.target
                    ));
                }
            }
            Err(err) => errors.push(format!(
                "{label} 的 target 不是合法 URL（{}）：{err}",
                instance.target
            )),
        }

        // httpPort 不能为 0，且实例之间不能重复
        if instance.http_port == 0 {
            errors.push(format!("{label} 的 httpPort 不能为 0"));
        } else if let Some(existing) = ports.insert(instance.http_port, label.clone()) {
            errors.push(format!(
                "{label} 的 httpPort {} 与 {existing} 重复",
                instance.http_port
            ));
        }

        // protocol 只能是 openai 或 anthropic
        if !SUPPORTED_PROTOCOLS.contains(&instance.protocol.as_str()) {
            errors.push(format!(
                "{label} 的 protocol 取值非法（{}），仅支持 openai 或 anthropic",
                instance.protocol
            ));
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 测试内自增序号，保证同一进程内目录名唯一（避免并行测试互相干扰）
    static DIR_SEQ: AtomicU64 = AtomicU64::new(0);

    /// 在系统临时目录下创建本次测试专用的唯一目录，避免污染仓库
    fn unique_temp_dir(tag: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX 纪元")
            .as_nanos();
        let seq = DIR_SEQ.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "simple-proxy-test-{tag}-{}-{nanos}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        dir
    }

    /// 构造一个代理实例
    fn instance(id: &str, name: &str, target: &str, port: u16) -> ProxyInstance {
        ProxyInstance {
            id: id.to_string(),
            name: name.to_string(),
            enabled: true,
            target: target.to_string(),
            http_port: port,
            protocol: "openai".to_string(),
            reasoning_effort: Some("max".to_string()),
            headers: BTreeMap::new(),
        }
    }

    /// 构造一份合法配置
    fn valid_config() -> AppConfig {
        AppConfig {
            version: 1,
            close_behavior: "minimize".to_string(),
            proxies: vec![
                instance("a", "实例 A", "https://example.com/v1", 8787),
                instance("b", "实例 B", "http://127.0.0.1:9000", 8788),
            ],
        }
    }

    #[test]
    fn 合法配置通过校验() {
        assert!(validate(&valid_config()).is_ok());
    }

    #[test]
    fn 非法target报错() {
        let mut config = valid_config();
        // 缺少协议头
        config.proxies[0].target = "example.com/v1".to_string();
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("target"), "错误信息应包含字段名: {err}");

        // 协议不受支持
        config.proxies[0].target = "ftp://example.com/v1".to_string();
        let err = validate(&config).expect_err("应校验失败");
        assert!(
            err.contains("协议不受支持"),
            "错误信息应说明协议问题: {err}"
        );
    }

    #[test]
    fn 端口冲突报错() {
        let mut config = valid_config();
        config.proxies[1].http_port = config.proxies[0].http_port;
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("httpPort"), "错误信息应包含字段名: {err}");
        assert!(err.contains("重复"), "错误信息应说明重复: {err}");
    }

    #[test]
    fn 端口为0报错() {
        let mut config = valid_config();
        config.proxies[0].http_port = 0;
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("不能为 0"), "错误信息应说明端口为 0: {err}");
    }

    #[test]
    fn 非法protocol报错() {
        let mut config = valid_config();
        config.proxies[0].protocol = "gemini".to_string();
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("protocol"), "错误信息应包含字段名: {err}");
    }

    #[test]
    fn 非法close_behavior报错() {
        let mut config = valid_config();
        config.close_behavior = "hide".to_string();
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("closeBehavior"), "错误信息应包含字段名: {err}");
    }

    #[test]
    fn id为空或重复报错() {
        let mut config = valid_config();
        config.proxies[0].id = "   ".to_string();
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("id"), "错误信息应包含字段名: {err}");

        let mut config = valid_config();
        config.proxies[1].id = config.proxies[0].id.clone();
        let err = validate(&config).expect_err("应校验失败");
        assert!(err.contains("重复"), "错误信息应说明重复: {err}");
    }

    #[test]
    fn 文件不存在时返回默认空配置() {
        let dir = unique_temp_dir("missing");
        let path = dir.join("config.json");

        let (config, warning) = load(&path).expect("读取缺失文件应成功");
        assert!(warning.is_none());
        assert_eq!(config.version, 1);
        assert_eq!(config.close_behavior, "minimize");
        assert!(config.proxies.is_empty());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 损坏json会备份并返回空配置与警告() {
        let dir = unique_temp_dir("broken");
        let path = dir.join("config.json");
        std::fs::write(&path, "{ 这不是合法 JSON").expect("写入损坏配置失败");

        let (config, warning) = load(&path).expect("读取损坏配置应成功返回空配置");
        let warning = warning.expect("应返回警告");
        assert!(warning.contains("配置已损坏"), "警告内容: {warning}");
        assert!(warning.contains("config.json.bak"), "警告内容: {warning}");
        assert!(config.proxies.is_empty());

        // 原文件已备份，且备份内容与损坏内容一致
        assert!(!path.exists(), "原文件应已被移走");
        let backup = dir.join("config.json.bak");
        assert!(backup.exists(), "应生成备份文件");
        let backup_text = std::fs::read_to_string(&backup).expect("读取备份失败");
        assert_eq!(backup_text, "{ 这不是合法 JSON");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 保存后读回内容一致() {
        let dir = unique_temp_dir("roundtrip");
        let path = dir.join("nested").join("config.json");

        let mut config = valid_config();
        config.proxies[0]
            .headers
            .insert("x-api-key".to_string(), "secret".to_string());
        config.proxies[1].reasoning_effort = None;

        save(&path, &config).expect("保存配置应成功");
        let text = std::fs::read_to_string(&path).expect("读取配置失败");
        assert!(
            text.contains("\"closeBehavior\""),
            "应为 camelCase 字段名: {text}"
        );
        assert!(text.contains("httpPort"), "应为 camelCase 字段名: {text}");

        let (loaded, warning) = load(&path).expect("读取配置应成功");
        assert!(warning.is_none());
        assert_eq!(loaded.version, config.version);
        assert_eq!(loaded.close_behavior, config.close_behavior);
        assert_eq!(loaded.proxies.len(), config.proxies.len());
        assert_eq!(loaded.proxies[0].id, config.proxies[0].id);
        assert_eq!(loaded.proxies[0].headers, config.proxies[0].headers);
        assert_eq!(loaded.proxies[0].http_port, config.proxies[0].http_port);
        assert_eq!(loaded.proxies[1].reasoning_effort, None);

        // 未配置的可选字段应能正常反序列化
        let parsed = serde_json::from_str::<AppConfig>(
            r#"{"proxies":[{"id":"x","name":"X","enabled":true,"target":"https://a.com","httpPort":8787}]}"#,
        )
        .expect("缺省字段应可省略");
        assert_eq!(parsed.version, 1);
        assert_eq!(parsed.close_behavior, "minimize");
        assert_eq!(parsed.proxies[0].protocol, "openai");
        assert_eq!(parsed.proxies[0].reasoning_effort, None);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn 保存非法配置会被拒绝() {
        let dir = unique_temp_dir("invalid-save");
        let path = dir.join("config.json");

        let mut config = valid_config();
        config.proxies[0].http_port = 0;
        let err = save(&path, &config).expect_err("非法配置不应写盘");
        assert!(err.contains("httpPort"), "错误信息: {err}");
        assert!(!path.exists(), "校验失败时不应生成文件");

        std::fs::remove_dir_all(&dir).ok();
    }
}
