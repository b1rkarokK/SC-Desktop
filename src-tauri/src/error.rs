use reqwest::StatusCode;
use serde::{ser::SerializeStruct, Serialize, Serializer};

pub type AppResult<T> = Result<T, AppError>;

/// Every error that can reach the UI. Messages are user-facing (Russian UI);
/// `kind` lets the frontend react programmatically (e.g. open Settings on auth errors).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Не заданы client_id и oauth_token. Откройте Настройки.")]
    NotAuthorized,
    #[error("Вход в SoundCloud устарел. Откройте Настройки и нажмите «Войти через SoundCloud».")]
    AuthExpired,
    #[error("Доступ запрещён (403). Если включён VPN или прокси — отключите: SoundCloud блокирует датацентровые IP.")]
    Forbidden,
    #[error("Сервер ограничил частоту запросов (429). Повторите позже.")]
    RateLimited,
    #[error("Достигнут суточный лимит потоков ({0} за 24 часа). Лимит SoundCloud — 15 000.")]
    StreamQuota(u32),
    #[error("Не найдено (404).")]
    NotFound,
    #[error("Ошибка сервера: HTTP {0}")]
    Http(u16),
    #[error("Сеть недоступна: {0}")]
    Network(String),
    #[error("Трек нельзя воспроизвести: {0}")]
    UnsupportedStream(String),
    #[error("Ошибка аудио: {0}")]
    Audio(String),
    #[error("Ошибка базы данных: {0}")]
    Db(String),
    #[error("Ошибка системного хранилища ключей: {0}")]
    Keyring(String),
    #[error("Не удалось разобрать ответ: {0}")]
    Parse(String),
    #[error("Некорректная настройка: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

impl AppError {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotAuthorized => "not_authorized",
            Self::AuthExpired => "auth_expired",
            Self::Forbidden => "forbidden",
            Self::RateLimited => "rate_limited",
            Self::StreamQuota(_) => "stream_quota",
            Self::NotFound => "not_found",
            Self::Http(_) => "http",
            Self::Network(_) => "network",
            Self::UnsupportedStream(_) => "unsupported_stream",
            Self::Audio(_) => "audio",
            Self::Db(_) => "db",
            Self::Keyring(_) => "keyring",
            Self::Parse(_) => "parse",
            Self::Config(_) => "config",
            Self::Other(_) => "other",
        }
    }

    pub fn from_status(status: StatusCode) -> Self {
        match status.as_u16() {
            401 => Self::AuthExpired,
            403 => Self::Forbidden,
            404 => Self::NotFound,
            429 => Self::RateLimited,
            code => Self::Http(code),
        }
    }

    /// Errors after which the player may silently move on to the next track.
    pub fn is_track_specific(&self) -> bool {
        matches!(self, Self::UnsupportedStream(_) | Self::NotFound | Self::Audio(_))
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("kind", self.kind())?;
        st.serialize_field("message", &self.to_string())?;
        st.end()
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        if let Some(status) = e.status() {
            return Self::from_status(status);
        }
        if e.is_decode() {
            return Self::Parse(e.without_url().to_string());
        }
        // without_url: never leak client_id / tokens from query strings into logs or UI
        Self::Network(e.without_url().to_string())
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        Self::Parse(e.to_string())
    }
}

impl From<url::ParseError> for AppError {
    fn from(e: url::ParseError) -> Self {
        Self::Parse(format!("URL: {e}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::Other(format!("I/O: {e}"))
    }
}

impl From<keyring::Error> for AppError {
    fn from(e: keyring::Error) -> Self {
        Self::Keyring(e.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(e: tauri::Error) -> Self {
        Self::Other(e.to_string())
    }
}
