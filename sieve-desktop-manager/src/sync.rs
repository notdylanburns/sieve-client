use std::collections::HashSet;

use log::{info, warn};
use reqwest::{StatusCode, header};

use crate::config::Config;
use crate::desktop::{self, Application};

#[derive(Debug)]
pub enum Error {
    /// The server refused the device token: revoked, or for another server.
    Unauthorized,
    Other(Box<dyn std::error::Error + Send + Sync>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "the server rejected the device token"),
            Self::Other(e) => e.fmt(f),
        }
    }
}

impl<E: Into<Box<dyn std::error::Error + Send + Sync>>> From<E> for Error {
    fn from(e: E) -> Self {
        Self::Other(e.into())
    }
}

fn check(response: reqwest::Response) -> Result<reqwest::Response, Error> {
    match response.status() {
        StatusCode::UNAUTHORIZED => Err(Error::Unauthorized),
        status if status.is_success() || status == StatusCode::NOT_MODIFIED => Ok(response),
        status => Err(format!("{} returned {status}", response.url()).into()),
    }
}

/// Keeps the local launchers in step with the workspace's application list.
pub struct Syncer {
    pub config: Config,
    http: reqwest::Client,
    /// The ETag of the list last installed, so an unchanged list costs one 304.
    installed: Option<Box<str>>,
}

impl Syncer {
    pub fn new(config: Config) -> Result<Self, reqwest::Error> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("sieve-desktop-manager/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(Self {
            config,
            http,
            installed: None,
        })
    }

    pub async fn sync(&mut self) -> Result<(), Error> {
        let mut request = self
            .http
            .get(self.config.api_url("/applications"))
            .bearer_auth(&self.config.device_token);
        if let Some(etag) = &self.installed {
            request = request.header(header::IF_NONE_MATCH, etag.as_ref());
        }

        let response = check(request.send().await?)?;
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(());
        }

        let etag: Option<Box<str>> = response
            .headers()
            .get(header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(Into::into);
        let applications: Vec<Application> = response.json().await?;
        info!("installing {} applications", applications.len());

        self.fetch_icons(&applications).await?;
        desktop::install(&self.config, &applications)?;

        // Only remembered once installed, so a failed install is retried on the next sync.
        self.installed = etag;
        Ok(())
    }

    /// Icon names are content hashes, so a file already on disk never needs fetching again.
    async fn fetch_icons(&self, applications: &[Application]) -> Result<(), Error> {
        let dir = desktop::icons_dir(&self.config);
        std::fs::create_dir_all(&dir)?;

        let wanted: HashSet<&str> = applications.iter().filter_map(desktop::icon_name).collect();

        for name in &wanted {
            let path = dir.join(name);
            if path.exists() {
                continue;
            }

            let url = self.config.server_url(&format!("/static/icons/{name}"));
            let response = self
                .http
                .get(url)
                .bearer_auth(&self.config.device_token)
                .send()
                .await
                .map_err(Error::from)
                .and_then(check);

            match response {
                Ok(response) => match response.bytes().await {
                    Ok(bytes) => desktop::write_atomically(&path, &bytes, 0o644)?,
                    Err(e) => warn!("failed to download icon {name}: {e}"),
                },
                Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                Err(e) => warn!("failed to download icon {name}: {e}"),
            }
        }

        for entry in dir.read_dir()?.flatten() {
            let name = entry.file_name();
            if !wanted.contains(name.to_string_lossy().as_ref()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }

        Ok(())
    }
}
