use std::sync::{atomic::AtomicBool, Arc, RwLock};

use tauri::{AppHandle, Manager};

use crate::{
    api::{
        lyrics::LyricsState,
        soundcloud::{ScUser, SoundCloud},
    },
    config::ConfigStore,
    db::{covers, CoverCache, Db},
    error::{AppError, AppResult},
    net::HttpClient,
    secrets::{self, Credentials},
    wave::WaveState,
};

pub struct AppState {
    pub db: Db,
    pub covers: CoverCache,
    pub config: ConfigStore,
    pub wave: WaveState,
    pub lyrics: LyricsState,
    pub likes_syncing: AtomicBool,
    /// «Категории» tile covers are being fetched
    pub covers_busy: AtomicBool,
    http: RwLock<Arc<HttpClient>>,
    creds: RwLock<Option<Credentials>>,
    me: RwLock<Option<ScUser>>,
}

fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(|p| p.into_inner()).clone()
}

fn write<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(|p| p.into_inner()) = value;
}

impl AppState {
    pub fn init(app: &AppHandle) -> AppResult<Self> {
        let paths = app.path();
        let data_dir = paths.app_data_dir()?;
        let cache_dir = paths.app_cache_dir()?;
        let config_dir = paths.app_config_dir()?;
        for d in [&data_dir, &cache_dir, &config_dir] {
            std::fs::create_dir_all(d)?;
        }

        let config = ConfigStore::load(config_dir.join("config.json"));
        crate::lang::set_from_ui(&config.get().ui);
        let http = match HttpClient::new(&config.get()) {
            Ok(c) => c,
            Err(e) => {
                // a broken proxy setting must not brick the app: fall back to direct
                tracing::error!(error = %e, "HTTP client init failed, falling back to direct connection");
                let mut cfg = config.get();
                cfg.net_mode = crate::config::NetMode::Direct;
                HttpClient::new(&cfg)?
            }
        };
        let db = Db::open(&data_dir.join("cache.sqlite3"))?;
        let covers = CoverCache::new(cache_dir.join("covers"), db.clone(), covers::MAX_BYTES)?;
        let creds = secrets::load().unwrap_or_else(|e| {
            tracing::error!(error = %e, "keyring read failed");
            None
        });

        let wave = WaveState::default();
        wave.set_no_liked(config.get().wave_no_liked);
        tracing::info!(data = %data_dir.display(), has_credentials = creds.is_some(), "state initialised");
        Ok(Self {
            db,
            covers,
            config,
            wave,
            lyrics: LyricsState::default(),
            likes_syncing: AtomicBool::new(false),
            covers_busy: AtomicBool::new(false),
            http: RwLock::new(Arc::new(http)),
            creds: RwLock::new(creds),
            me: RwLock::new(None),
        })
    }

    pub fn http(&self) -> Arc<HttpClient> {
        read(&self.http)
    }

    pub fn rebuild_http(&self) -> AppResult<()> {
        let client = HttpClient::new(&self.config.get())?;
        write(&self.http, Arc::new(client));
        Ok(())
    }

    pub fn credentials(&self) -> Option<Credentials> {
        read(&self.creds)
    }

    pub fn set_credentials(&self, creds: Option<Credentials>) {
        write(&self.creds, creds);
        write(&self.me, None);
    }

    pub fn me(&self) -> Option<ScUser> {
        read(&self.me)
    }

    pub fn set_me(&self, me: ScUser) {
        write(&self.me, Some(me));
    }

    pub fn sc(&self) -> AppResult<SoundCloud> {
        let mut creds = self.credentials().ok_or(AppError::NotAuthorized)?;
        // SoundCloud rotated the public client_id or renewed the login token:
        // use and keep the fresh ones, the user never has to sign in again
        let fresh_id = crate::api::soundcloud::rotated_client_id().filter(|id| *id != creds.client_id);
        let fresh_token = crate::api::soundcloud::rotated_token().filter(|t| *t != creds.oauth_token);
        if fresh_id.is_some() || fresh_token.is_some() {
            if let Some(id) = fresh_id {
                creds.client_id = id;
            }
            if let Some(t) = fresh_token {
                creds.oauth_token = t;
            }
            self.set_credentials_keep_me(creds.clone());
            let to_save = creds.clone();
            std::thread::spawn(move || {
                if let Err(e) = crate::secrets::save(&to_save) {
                    tracing::warn!(error = %e, "saving the fresh client_id failed");
                }
            });
        }
        Ok(SoundCloud::new(self.http(), &creds))
    }

    /// Same account, new client_id: the cached `/me` stays valid.
    fn set_credentials_keep_me(&self, creds: Credentials) {
        write(&self.creds, Some(creds));
    }

    /// Cached `/me`, fetched once per session.
    pub async fn current_user(&self) -> AppResult<ScUser> {
        if let Some(me) = self.me() {
            return Ok(me);
        }
        let me = self.sc()?.me().await?;
        self.set_me(me.clone());
        Ok(me)
    }
}
