use url::Url;

/// 解析出的 `dbstudio://` 深链。
///
/// 只带 `connection_id` 与 `nonce`，**不携带任何凭据**。旧格式会把
/// `password=` 直接拼进 URL，于是密码会留在进程参数（`ps` 可见）、shell 历史、
/// 系统事件日志和浏览器/邮件的转发记录里；现在凭据留在本机 keyring，由 dbstudio
/// 自己按 `connection_id` 取回。
///
/// `connection_id` 指向本机已保存的连接，所以深链只能在本机生效——这正是想要的
/// 语义：它的作用是"让别的程序唤起我打开某个连接"，不是跨机器分发凭据。
/// `nonce` 存在 keyring（与密码同一层），删除连接时一并清除，因此删除即吊销。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbStudioUrl {
    pub action: String,
    pub connection_id: String,
    pub nonce: String,
}

impl DbStudioUrl {
    /// Parse a dbstudio:// URL from a string.
    ///
    /// 缺 `id` 直接拒绝（无从定位连接）。`nonce` 允许缺失，交给处理端报错——
    /// 静默丢弃一个"看起来像深链"的启动参数只会让排障变得更难。
    pub fn parse(url_str: &str) -> Option<Self> {
        let url = Url::parse(url_str).ok()?;
        if url.scheme() != "dbstudio" {
            return None;
        }

        let action = url.host_str()?.to_string();
        let params: std::collections::HashMap<String, String> =
            url.query_pairs().into_owned().collect();
        let connection_id = params.get("id")?.trim().to_string();
        if connection_id.is_empty() {
            return None;
        }
        let nonce = params
            .get("nonce")
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        Some(Self {
            action,
            connection_id,
            nonce,
        })
    }

    /// Check if this is a connect action.
    pub fn is_connect(&self) -> bool {
        self.action == "connect"
    }
}

/// 为一个已保存连接构造深链。`nonce` 由 `ConnectionsRepository::ensure_link_nonce`
/// 生成并存在 keyring 里；这里只做拼接。
pub fn build_url(connection_id: &str, nonce: &str) -> String {
    format!("dbstudio://connect?id={connection_id}&nonce={nonce}")
}

/// 校验深链 nonce。逐字节异或累加而不是短路比较，避免把令牌长度或首个不匹配
/// 字节的位置泄露进时序——深链来自命令行参数，本机任何人都能读到。
pub fn nonce_matches(expected: &str, actual: &str) -> bool {
    if expected.is_empty() || expected.len() != actual.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in expected.bytes().zip(actual.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Check command-line arguments for a dbstudio:// URL.
pub fn find_url_in_args() -> Option<String> {
    std::env::args().find(|arg| arg.starts_with("dbstudio://"))
}

// Inline tests extracted to url_scheme_tests.rs.
#[cfg(test)]
#[path = "url_scheme_tests.rs"]
mod url_scheme_tests;
