use std::path::{Path, PathBuf};

use serde::{Serialize, Serializer};

pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Unified error type exposed by cairn-core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("网络请求失败({url}):{source}")]
    Http {
        url: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("服务器返回 HTTP {status}({url})")]
    HttpStatus { url: String, status: u16 },
    #[error("文件操作失败({path}):{source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JSON 解析失败({context}):{source}")]
    Json {
        context: String,
        #[source]
        source: serde_json::Error,
    },
    #[error("文件校验失败:{path}(期望 {expected},实际 {actual})")]
    HashMismatch { path: PathBuf, expected: String, actual: String },
    #[error("下载失败:{0}")]
    Download(String),
    #[error("压缩包处理失败({path}):{message}")]
    Archive { path: PathBuf, message: String },
    #[error("操作已取消")]
    Cancelled,
    #[error("找不到{0}")]
    NotFound(String),
    #[error("{0}")]
    Invalid(String),
    #[error("账号错误:{0}")]
    Auth(String),
    #[error("Java 错误:{0}")]
    Java(String),
    #[error("安装失败:{0}")]
    Install(String),
    #[error("启动失败:{0}")]
    Launch(String),
    #[error("缺少配置项 {0},请在配置中填写或在编译时通过环境变量注入")]
    MissingConfig(&'static str),
    #[error("系统调用失败:{0}")]
    Platform(String),
}

/// Error categories used by the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    Network,
    Io,
    Data,
    Cancelled,
    NotFound,
    Invalid,
    Auth,
    Java,
    Install,
    Launch,
    Config,
    Platform,
}

impl Error {
    pub fn kind(&self) -> ErrorKind {
        match self {
            Error::Http { .. } | Error::HttpStatus { .. } | Error::Download(_) => ErrorKind::Network,
            Error::Io { .. } => ErrorKind::Io,
            Error::Json { .. } | Error::HashMismatch { .. } | Error::Archive { .. } => ErrorKind::Data,
            Error::Cancelled => ErrorKind::Cancelled,
            Error::NotFound(_) => ErrorKind::NotFound,
            Error::Invalid(_) => ErrorKind::Invalid,
            Error::Auth(_) => ErrorKind::Auth,
            Error::Java(_) => ErrorKind::Java,
            Error::Install(_) => ErrorKind::Install,
            Error::Launch(_) => ErrorKind::Launch,
            Error::MissingConfig(_) => ErrorKind::Config,
            Error::Platform(_) => ErrorKind::Platform,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        matches!(self, Error::Cancelled)
    }

    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io { path: path.into(), source }
    }

    pub(crate) fn json(context: impl Into<String>, source: serde_json::Error) -> Self {
        Error::Json { context: context.into(), source }
    }

    pub(crate) fn http(url: impl Into<String>, source: reqwest::Error) -> Self {
        Error::Http { url: url.into(), source }
    }

    pub(crate) fn archive(path: impl Into<PathBuf>, message: impl ToString) -> Self {
        Error::Archive { path: path.into(), message: message.to_string() }
    }
}

impl Serialize for Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut s = serializer.serialize_struct("Error", 2)?;
        s.serialize_field("kind", &self.kind())?;
        s.serialize_field("message", &self.to_string())?;
        s.end()
    }
}

/// Attaches a file path context to an `io::Result`.
pub(crate) trait IoContext<T> {
    fn ctx(self, path: &Path) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn ctx(self, path: &Path) -> Result<T> {
        self.map_err(|e| Error::io(path, e))
    }
}
