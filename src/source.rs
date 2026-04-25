use std::time::Duration;

use bson::doc;
use mongodb::options::{
    ClientOptions, ReadPreference, ReadPreferenceOptions, SelectionCriteria, Tls, TlsOptions,
};
use mongodb::{Client, Database};

use crate::bson_ext::{FlatMetric, flatten_bson};

/// One poll's worth of metrics, paired with the host that returned them and the
/// server-reported version (used for the header).
#[derive(Debug, Clone)]
pub struct Sample {
    pub metrics: Vec<FlatMetric>,
    pub host: Option<String>,
    pub version: Option<String>,
}

/// How a poll failed. Transient errors keep the polling loop alive and surface
/// a "reconnecting" banner; fatal errors stop polling.
#[derive(Debug)]
pub enum PollError {
    /// Network/server-selection/pool errors — driver will retry on next poll.
    Transient(String),
    /// Auth errors and similar — no point retrying without operator intervention.
    Fatal(String),
}

impl std::fmt::Display for PollError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PollError::Transient(msg) => write!(f, "transient: {msg}"),
            PollError::Fatal(msg) => write!(f, "fatal: {msg}"),
        }
    }
}

impl std::error::Error for PollError {}

/// Abstract source of metric samples. Phase 1 ships only `ServerStatusSource`;
/// future impls can poll replSetGetStatus, Atlas, etc.
#[allow(async_fn_in_trait)]
pub trait MetricSource {
    async fn poll(&mut self) -> Result<Sample, PollError>;
}

/// Polls `serverStatus` against the admin database on every tick.
pub struct ServerStatusSource {
    db: Database,
    selection: SelectionCriteria,
    /// Display label for the connection (URI sans credentials, or first host:port).
    pub label: String,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ReadPref {
    Primary,
    PrimaryPreferred,
    Secondary,
    SecondaryPreferred,
    Nearest,
}

impl ReadPref {
    fn into_selection(self) -> SelectionCriteria {
        let opts: Option<ReadPreferenceOptions> = None;
        let pref = match self {
            ReadPref::Primary => ReadPreference::Primary,
            ReadPref::PrimaryPreferred => ReadPreference::PrimaryPreferred { options: opts },
            ReadPref::Secondary => ReadPreference::Secondary { options: opts },
            ReadPref::SecondaryPreferred => ReadPreference::SecondaryPreferred { options: opts },
            ReadPref::Nearest => ReadPreference::Nearest { options: opts },
        };
        SelectionCriteria::ReadPreference(pref)
    }
}

pub struct SourceConfig {
    pub uri: String,
    pub app_name: String,
    pub connect_timeout: Duration,
    pub server_selection_timeout: Duration,
    pub read_pref: ReadPref,
    pub tls_allow_invalid_certs: bool,
}

impl ServerStatusSource {
    pub async fn connect(cfg: SourceConfig) -> Result<Self, PollError> {
        let mut opts = ClientOptions::parse(&cfg.uri)
            .await
            .map_err(|e| PollError::Fatal(format!("parse uri: {e}")))?;

        opts.app_name = Some(cfg.app_name);
        opts.connect_timeout = Some(cfg.connect_timeout);
        opts.server_selection_timeout = Some(cfg.server_selection_timeout);
        // 1Hz polling — bump heartbeat down so SDAM marks dead nodes quickly.
        opts.heartbeat_freq = Some(Duration::from_secs(1));

        if cfg.tls_allow_invalid_certs {
            let mut tls = TlsOptions::default();
            tls.allow_invalid_certificates = Some(true);
            opts.tls = Some(Tls::Enabled(tls));
        }

        let label = label_from_options(&opts, &cfg.uri);

        let client =
            Client::with_options(opts).map_err(|e| PollError::Fatal(format!("client: {e}")))?;

        Ok(ServerStatusSource {
            db: client.database("admin"),
            selection: cfg.read_pref.into_selection(),
            label,
        })
    }
}

impl MetricSource for ServerStatusSource {
    async fn poll(&mut self) -> Result<Sample, PollError> {
        let result = self
            .db
            .run_command(doc! { "serverStatus": 1 })
            .selection_criteria(self.selection.clone())
            .await;

        let doc = match result {
            Ok(d) => d,
            Err(e) => return Err(classify_error(&e)),
        };

        let host = doc.get_str("host").ok().map(str::to_string);
        let version = doc.get_str("version").ok().map(str::to_string);
        let metrics = flatten_bson(&doc);

        Ok(Sample {
            metrics,
            host,
            version,
        })
    }
}

fn classify_error(e: &mongodb::error::Error) -> PollError {
    use mongodb::error::ErrorKind;
    match &*e.kind {
        ErrorKind::Authentication { .. } => PollError::Fatal(e.to_string()),
        ErrorKind::Command(cmd) => {
            // Unauthorized (13) / AuthenticationFailed (18) → fatal.
            if matches!(cmd.code, 13 | 18) {
                PollError::Fatal(e.to_string())
            } else {
                PollError::Transient(e.to_string())
            }
        }
        // Network/topology issues are transient — the driver will retry on next poll.
        ErrorKind::Io(_)
        | ErrorKind::ConnectionPoolCleared { .. }
        | ErrorKind::ServerSelection { .. }
        | ErrorKind::DnsResolve { .. }
        | ErrorKind::Internal { .. } => PollError::Transient(e.to_string()),
        // Default: treat as transient. Banner shows the reason; counters drop a tick.
        _ => PollError::Transient(e.to_string()),
    }
}

fn label_from_options(opts: &ClientOptions, fallback_uri: &str) -> String {
    if let Some(host) = opts.hosts.first() {
        return host.to_string();
    }
    // Strip credentials so we never echo a password on screen.
    if let Some((scheme, rest)) = fallback_uri.split_once("://") {
        if let Some((_, host)) = rest.split_once('@') {
            return format!("{scheme}://{host}");
        }
        return format!("{scheme}://{rest}");
    }
    fallback_uri.to_string()
}
