use std::sync::Arc;

use deadpool_postgres::{Manager, ManagerConfig, Pool, RecyclingMethod};
use serde::{Deserialize, Serialize};
use tokio_postgres::{Client, Config, NoTls};
use tokio_postgres_rustls::MakeRustlsConnect;

use crate::error::{AppError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SslMode {
    Disable,
    #[default]
    Prefer,
    Require,
}

/// A saved connection target. Never contains a password — those live in the OS
/// keychain, keyed by profile id (see `crate::secrets`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    #[serde(default)]
    pub sslmode: SslMode,
}

impl Profile {
    /// Parse a libpq-style URL, used by `PGSCOPE_DEV_URL` for dev auto-connect.
    /// Returns the profile plus the password embedded in the URL, if any.
    ///
    /// # Arguments
    /// * `url` — `&str`: a libpq connection string or `postgres://` URL, possibly
    ///   containing a password.
    /// * `id` — `&str`: profile id to assign; also the keychain key.
    /// * `name` — `&str`: display name for the profile.
    ///
    /// # Returns
    /// `Result<(Self, Option<String>)>` — the profile, always with
    /// `SslMode::Prefer`, and the URL's password if it carried one. `Err` is
    /// `AppError::Invalid` when the URL won't parse or names no database or user.
    pub fn from_url(url: &str, id: &str, name: &str) -> Result<(Self, Option<String>)> {
        let cfg: Config = url
            .parse()
            .map_err(|e| AppError::Invalid(format!("bad connection URL: {e}")))?;

        let host = match cfg.get_hosts().first() {
            Some(tokio_postgres::config::Host::Tcp(h)) => h.clone(),
            _ => "localhost".to_string(),
        };
        let port = cfg.get_ports().first().copied().unwrap_or(5432);
        let database = cfg
            .get_dbname()
            .ok_or_else(|| AppError::Invalid("connection URL has no database".into()))?
            .to_string();
        let user = cfg
            .get_user()
            .ok_or_else(|| AppError::Invalid("connection URL has no user".into()))?
            .to_string();
        let password = cfg
            .get_password()
            .map(|p| String::from_utf8_lossy(p).into_owned());

        Ok((
            Self {
                id: id.to_string(),
                name: name.to_string(),
                host,
                port,
                database,
                user,
                sslmode: SslMode::Prefer,
            },
            password,
        ))
    }
}

/// What the UI needs to render the titlebar and pick the psql prompt suffix.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub database: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub server_version: String,
    pub is_superuser: bool,
}

/// Session options for the browse pool: reads only, and never hangs the UI.
///
/// This is the guarantee that grid browsing and introspection can't mutate
/// anything — writes are the terminal's job, on its own unrestricted client.
const BROWSE_OPTIONS: &str = "-c default_transaction_read_only=on -c statement_timeout=30s";

/// Everything a connection needs except TLS, application name, and session
/// options — the parts that differ between the browse pool and the terminal.
///
/// A `None` password means rely on the server's own auth (trust, peer, or a
/// pgpass file); the keychain lookup happens before we get here.
///
/// # Arguments
/// * `profile` — `&Profile`: host, port, database, and user; `sslmode` is *not*
///   applied here, callers branch on it themselves.
/// * `password` — `Option<&str>`: `None` leaves the password unset entirely.
///
/// # Returns
/// `Config` — a fresh config the caller is expected to extend with application
/// name, session options, and a TLS choice.
fn base_config(profile: &Profile, password: Option<&str>) -> Config {
    let mut cfg = Config::new();
    cfg.host(&profile.host)
        .port(profile.port)
        .dbname(&profile.database)
        .user(&profile.user);
    if let Some(pw) = password {
        cfg.password(pw);
    }
    cfg
}

/// TLS using the bundled webpki root store, with no client certificate.
///
/// Roots are compiled in rather than read from the OS, so a self-signed server
/// cert will fail verification here and `sslmode=prefer` will quietly drop to
/// plaintext — `require` is what surfaces the error.
///
/// # Arguments
/// None.
///
/// # Returns
/// `MakeRustlsConnect` — a connector trusting only the compiled-in webpki roots,
/// built fresh on each call.
fn tls_connector() -> MakeRustlsConnect {
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    MakeRustlsConnect::new(config)
}

/// Connect a single client, spawning its connection driver task.
///
/// `sslmode=prefer` mirrors libpq: try TLS, fall back to plaintext if the
/// server refuses it. `require` fails instead of falling back.
///
/// # Arguments
/// * `profile` — `&Profile`: connection target; its `sslmode` selects the
///   strategy.
/// * `password` — `Option<&str>`: `None` defers to server-side auth.
/// * `application_name` — `&str`: shown in `pg_stat_activity`, e.g.
///   `pgscope-probe` or the terminal's name.
/// * `options` — `Option<&str>`: raw `-c key=value` session options, or `None`
///   for an unrestricted session.
///
/// # Returns
/// `Result<Client>` — a live client whose driver task is already spawned. `Err`
/// covers refused connections, auth failures, and — under `require` — refused
/// TLS.
pub async fn connect_client(
    profile: &Profile,
    password: Option<&str>,
    application_name: &str,
    options: Option<&str>,
) -> Result<Client> {
    let mut cfg = base_config(profile, password);
    cfg.application_name(application_name);
    if let Some(opts) = options {
        cfg.options(opts);
    }

    match profile.sslmode {
        SslMode::Disable => spawn_plain(cfg).await,
        SslMode::Require => spawn_tls(cfg).await,
        SslMode::Prefer => match spawn_tls(cfg.clone()).await {
            Ok(c) => Ok(c),
            Err(_) => spawn_plain(cfg).await,
        },
    }
}

/// Connect without TLS, detaching the driver task that pumps the socket.
///
/// The task owns the connection half and outlives this call; dropping the
/// returned client is what ends it. A driver error can only be logged, since by
/// then nobody is awaiting this future.
///
/// # Arguments
/// * `cfg` — `Config`: taken by value, fully populated; nothing further is set
///   here.
///
/// # Returns
/// `Result<Client>` — the client half. `Err` if the TCP connect or startup
/// handshake fails; later driver errors are only logged.
async fn spawn_plain(cfg: Config) -> Result<Client> {
    let (client, connection) = cfg.connect(NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("pgscope: connection closed: {e}");
        }
    });
    Ok(client)
}

/// As [`spawn_plain`], but negotiating TLS first.
///
/// Under `sslmode=prefer` the caller discards this error and retries in the
/// clear, so a failure here is not necessarily reported anywhere.
///
/// # Arguments
/// * `cfg` — `Config`: taken by value; under `prefer` the caller keeps a clone
///   for the plaintext retry.
///
/// # Returns
/// `Result<Client>` — the client half. `Err` additionally covers certificate
/// verification failure and a server that refuses TLS.
async fn spawn_tls(cfg: Config) -> Result<Client> {
    let (client, connection) = cfg.connect(tls_connector()).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("pgscope: connection closed: {e}");
        }
    });
    Ok(client)
}

/// Build the pooled, read-only browse connection used for introspection and
/// grid paging.
///
/// # Arguments
/// * `profile` — `&Profile`: connection target; `sslmode` picks TLS or plain,
///   with no per-connection `prefer` fallback.
/// * `password` — `Option<&str>`: `None` defers to server-side auth.
///
/// # Returns
/// `Result<Pool>` — a pool capped at 4 connections, every one of them carrying
/// `BROWSE_OPTIONS`. `Err` is `AppError::Connect` if the pool can't be built;
/// note no connection is attempted here, so a bad host fails later.
pub fn build_browse_pool(profile: &Profile, password: Option<&str>) -> Result<Pool> {
    let mut cfg = base_config(profile, password);
    cfg.application_name("pgscope");
    cfg.options(BROWSE_OPTIONS);

    let mgr_cfg = ManagerConfig {
        recycling_method: RecyclingMethod::Fast,
    };

    let mgr = match profile.sslmode {
        SslMode::Disable => Manager::from_config(cfg, NoTls, mgr_cfg),
        // For pooling we don't do prefer-style fallback per connection; the
        // initial probe in `connect()` has already established what works.
        _ => {
            return Pool::builder(Manager::from_config(cfg, tls_connector(), mgr_cfg))
                .max_size(4)
                .build()
                .map_err(|e| AppError::Connect(e.to_string()));
        }
    };

    Pool::builder(mgr)
        .max_size(4)
        .build()
        .map_err(|e| AppError::Connect(e.to_string()))
}

/// Probe a connection and collect the facts the UI displays.
///
/// # Arguments
/// * `client` — `&Client`: an already-connected client, typically the throwaway
///   probe client rather than a pooled one.
/// * `profile` — `&Profile`: supplies host, port, and database, which are echoed
///   rather than asked of the server.
///
/// # Returns
/// `Result<ConnectionInfo>` — server version plus the *effective* user and its
/// superuser flag, both read from the server. `Err` if the probe query fails.
pub async fn probe(client: &Client, profile: &Profile) -> Result<ConnectionInfo> {
    let row = client
        .query_one(
            "SELECT current_setting('server_version'), \
                    pg_catalog.current_user()::text, \
                    COALESCE((SELECT r.rolsuper FROM pg_roles r \
                              WHERE r.rolname = current_user), false)",
            &[],
        )
        .await?;

    Ok(ConnectionInfo {
        database: profile.database.clone(),
        host: profile.host.clone(),
        port: profile.port,
        user: row.get::<_, String>(1),
        server_version: row.get::<_, String>(0),
        is_superuser: row.get::<_, bool>(2),
    })
}

/// An established connection: a read-only pool for browsing plus the facts
/// about the server. The terminal's client is tracked separately.
pub struct Connection {
    pub profile: Profile,
    pub password: Option<String>,
    pub pool: Pool,
    pub info: ConnectionInfo,
}

impl Connection {
    /// Establish a connection: resolve TLS, read the server facts, and stand up
    /// the browse pool.
    ///
    /// Everything that can go wrong — bad host, bad credentials, TLS refused —
    /// is made to fail here rather than on the first grid page, so the connect
    /// dialog is where the user sees it. The password is retained because the
    /// terminal opens its own unrestricted client later.
    ///
    /// # Arguments
    /// * `profile` — `Profile`: taken by value and stored on the connection.
    /// * `password` — `Option<String>`: retained for the terminal's later client;
    ///   `None` means server-side auth is expected to suffice.
    ///
    /// # Returns
    /// `Result<Arc<Self>>` — shared because the browse pool and profile outlive
    /// any single command handler. `Err` if the probe, the pool build, or the
    /// first pooled checkout fails.
    pub async fn open(profile: Profile, password: Option<String>) -> Result<Arc<Self>> {
        // Probe first with a plain client so sslmode=prefer fallback is
        // resolved (and auth errors surface) before we build the pool.
        let probe_client =
            connect_client(&profile, password.as_deref(), "pgscope-probe", None).await?;
        let info = probe(&probe_client, &profile).await?;
        drop(probe_client);

        let pool = build_browse_pool(&profile, password.as_deref())?;
        // Fail fast if the pooled configuration can't actually connect.
        let _ = pool.get().await?;

        Ok(Arc::new(Self {
            profile,
            password,
            pool,
            info,
        }))
    }

    /// Round-trip latency in milliseconds, for the titlebar pill.
    ///
    /// # Arguments
    /// * `&self` — measured against this connection's browse pool, so the timing
    ///   includes waiting for a free pooled connection.
    ///
    /// # Returns
    /// `Result<f64>` — elapsed milliseconds for a `SELECT 1`. `Err` if no
    /// connection can be checked out or the round trip fails.
    pub async fn ping(&self) -> Result<f64> {
        let client = self.pool.get().await?;
        let t0 = std::time::Instant::now();
        client.simple_query("SELECT 1").await?;
        Ok(t0.elapsed().as_secs_f64() * 1000.0)
    }
}
