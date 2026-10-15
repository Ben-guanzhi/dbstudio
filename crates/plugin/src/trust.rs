use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Trust store file name, next to the plugin directories.
const PLUGIN_TRUST_FILE: &str = "plugin_trust.json";

/// 一条信任记录：插件文件的规范路径 + 登记时的 SHA-256。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TrustedPlugin {
    pub path: String,
    pub sha256: String,
}

/// `plugin_trust.json` 的内容。
#[derive(Debug, Default, Serialize, Deserialize)]
struct TrustStore {
    #[serde(default)]
    trusted: Vec<TrustedPlugin>,
}

/// 一个插件文件的信任判定结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustDecision {
    /// 在清单里，且内容与登记时一致。
    Trusted,
    /// 清单里没有这个路径。
    Unknown,
    /// 清单里有，但文件内容已经变了。
    Tampered { expected: String, actual: String },
}

/// 打开一个插件库之前必须先过的信任门。
///
/// `libloading::Library::new` 会立刻执行目标 DLL/SO 的初始化代码，所以信任判定
/// 必须发生在打开文件**之前**——读字节算摘要本身是安全的，`dlopen` 不是。
///
/// 清单只记文件自己的摘要，不记签名：本机没有可验的公钥。它挡住的是"插件目录
/// 里多了个文件"和"已登记的文件被换掉"这两种情况。要验发布者身份，得插件方
/// 自己把签名带上。
pub struct PluginTrust {
    path: PathBuf,
}

impl PluginTrust {
    /// 默认清单位置，与插件目录同根：`data_local_dir()/dbstudio/plugin_trust.json`。
    pub fn at_default() -> Self {
        let root = dirs::data_local_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("dbstudio");
        Self::at(root.join(PLUGIN_TRUST_FILE))
    }

    pub fn at(path: PathBuf) -> Self {
        Self { path }
    }

    /// 清单文件路径，用于在报错里告诉用户去哪儿改。
    pub fn file_path(&self) -> &Path {
        &self.path
    }

    /// 计算文件的 SHA-256（小写十六进制）与字节数。只读文件内容，不执行代码。
    pub fn fingerprint(path: &Path) -> Result<(String, u64)> {
        let bytes =
            fs::read(path).with_context(|| format!("无法读取插件文件：{}", path.display()))?;
        let digest = Sha256::digest(&bytes);
        Ok((to_hex(digest), bytes.len() as u64))
    }

    /// 读清单。文件不存在等于"还没有信任任何插件"，是正常状态而不是错误。
    fn load_store(&self) -> Result<TrustStore> {
        match fs::read_to_string(&self.path) {
            Ok(text) if text.trim().is_empty() => Ok(TrustStore::default()),
            Ok(text) => serde_json::from_str(&text)
                .with_context(|| format!("信任清单不是合法 JSON：{}", self.path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(TrustStore::default()),
            Err(e) => Err(e).with_context(|| format!("无法读取信任清单：{}", self.path.display())),
        }
    }

    /// 写清单：先写临时文件再改名，中途失败不会留下半截 JSON。
    fn save_store(&self, store: &TrustStore) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("无法创建信任清单目录：{}", parent.display()))?;
        }

        let json = serde_json::to_string_pretty(store)
            .with_context(|| "序列化信任清单失败".to_string())?;
        let tmp = self.path.with_extension("tmp");
        fs::write(&tmp, json.as_bytes())
            .with_context(|| format!("无法写入临时信任清单：{}", tmp.display()))?;
        fs::rename(&tmp, &self.path)
            .with_context(|| format!("无法更新信任清单：{}", self.path.display()))
    }

    /// 判定一个文件是否可信。
    pub fn check(&self, path: &Path) -> Result<TrustDecision> {
        let (actual, _) = Self::fingerprint(path)?;
        let store = self.load_store()?;

        let Some(entry) = store
            .trusted
            .iter()
            .find(|entry| same_path(&entry.path, path))
        else {
            return Ok(TrustDecision::Unknown);
        };

        if entry.sha256.eq_ignore_ascii_case(&actual) {
            Ok(TrustDecision::Trusted)
        } else {
            Ok(TrustDecision::Tampered {
                expected: entry.sha256.clone(),
                actual,
            })
        }
    }

    /// 把文件登记进信任清单，返回登记进去的摘要。重复登记会刷新摘要。
    pub fn trust(&self, path: &Path) -> Result<String> {
        let (hash, _) = Self::fingerprint(path)?;
        let key = canonical(path)?;
        let mut store = self.load_store()?;
        store
            .trusted
            .retain(|entry| !same_path_str(&entry.path, &key));
        store.trusted.push(TrustedPlugin {
            path: key.to_string_lossy().into_owned(),
            sha256: hash.clone(),
        });
        self.save_store(&store)?;
        Ok(hash)
    }

    /// 从信任清单里移除，让下一次加载重新回到"未知文件"。
    pub fn revoke(&self, path: &Path) -> Result<()> {
        let key = canonical(path)?;
        let mut store = self.load_store()?;
        store
            .trusted
            .retain(|entry| !same_path_str(&entry.path, &key));
        self.save_store(&store)
    }

    pub fn list(&self) -> Result<Vec<TrustedPlugin>> {
        Ok(self.load_store()?.trusted)
    }
}

/// 打开插件库前必须先过的信任判定。
///
/// 失败信息里带上清单位置，这样用户不用翻文档就知道下一步该做什么。
pub fn require_trusted(trust: &PluginTrust, path: &Path) -> Result<()> {
    match trust.check(path)? {
        TrustDecision::Trusted => Ok(()),
        TrustDecision::Unknown => Err(anyhow!(
            "插件 {} 不在信任清单里，已拒绝加载。\
             确认这个文件确实是你想装的那个，再把它登记进 {}（或删掉它）",
            path.display(),
            trust.file_path().display(),
        )),
        TrustDecision::Tampered { expected, actual } => Err(anyhow!(
            "插件 {} 的内容与信任清单不一致（清单 {}，当前 {}），已拒绝加载。\
             文件可能被替换或被篡改；如果这是有意的更新，请重新登记它",
            path.display(),
            hash_prefix(&expected),
            hash_prefix(&actual),
        )),
    }
}

fn to_hex(bytes: impl AsRef<[u8]>) -> String {
    let mut out = String::with_capacity(bytes.as_ref().len() * 2);
    for byte in bytes.as_ref() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// 只用于报错信息：完整 64 位摘要在日志里很难比对。
fn hash_prefix(hash: &str) -> String {
    hash.get(..12).unwrap_or(hash).to_string()
}

/// 规范路径，用于把登记时的路径和检查时的路径对上（相对/绝对写法不同）。
fn canonical(path: &Path) -> Result<PathBuf> {
    fs::canonicalize(path).with_context(|| format!("无法解析插件路径：{}", path.display()))
}

/// 比较一条已登记的规范路径和一个还没规范化的路径。
fn same_path(stored: &str, path: &Path) -> bool {
    match canonical(path) {
        Ok(key) => same_path_str(stored, &key),
        Err(_) => false,
    }
}

fn same_path_str(stored: &str, key: &Path) -> bool {
    stored.eq_ignore_ascii_case(&key.to_string_lossy())
}

#[cfg(test)]
#[path = "trust_tests.rs"]
mod tests;
