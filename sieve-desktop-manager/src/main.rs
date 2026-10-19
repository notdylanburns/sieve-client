//! Keeps a filtered user's launcher entries in step with their Sieve workspace: it listens on the
//! server's events WebSocket and reinstalls the application list whenever it may have changed.

use std::process::ExitCode;
use std::time::Duration;

use futures_util::StreamExt;
use log::{error, info, warn};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderValue, StatusCode, header};
use tokio_tungstenite::tungstenite::{self, Message};

use crate::config::Config;
use crate::sync::{Error, Syncer};

mod config;
mod desktop;
mod sync;

const MIN_RETRY: Duration = Duration::from_secs(1);
const MAX_RETRY: Duration = Duration::from_secs(60);
/// The server pings every 30 s, so this long without any frame means the connection is dead.
const IDLE_TIMEOUT: Duration = Duration::from_secs(75);

#[derive(serde::Deserialize)]
struct Event {
    r#type: Box<str>,
}

/// Follows one events connection until it ends. Returns whether the server greeted us, which
/// means the connection was healthy and the retry delay can start over.
async fn follow_events(syncer: &mut Syncer) -> Result<bool, Error> {
    let mut request = syncer.config.events_url().as_str().into_client_request()?;
    let bearer = HeaderValue::from_str(&format!("Bearer {}", syncer.config.device_token))?;
    request.headers_mut().insert(header::AUTHORIZATION, bearer);

    let (mut socket, _) = match tokio_tungstenite::connect_async(request).await {
        Ok(connected) => connected,
        Err(tungstenite::Error::Http(response)) if response.status() == StatusCode::UNAUTHORIZED => {
            return Err(Error::Unauthorized);
        }
        Err(e) => return Err(e.into()),
    };
    info!("connected to {}", syncer.config.server);

    let mut greeted = false;
    loop {
        let message = match tokio::time::timeout(IDLE_TIMEOUT, socket.next()).await {
            Err(_) => return Err(format!("no message from the server for {IDLE_TIMEOUT:?}").into()),
            Ok(None) | Ok(Some(Ok(Message::Close(_)))) => return Ok(greeted),
            Ok(Some(Err(e))) => return Err(e.into()),
            Ok(Some(Ok(message))) => message,
        };

        let Message::Text(text) = message else {
            continue;
        };

        match serde_json::from_str::<Event>(&text) {
            Ok(event) if matches!(event.r#type.as_ref(), "hello" | "applications_changed") => {
                // Every (re)connect starts with hello, so anything missed while away is picked up.
                greeted |= event.r#type.as_ref() == "hello";
                match syncer.sync().await {
                    Err(Error::Unauthorized) => return Err(Error::Unauthorized),
                    Err(e) => warn!("failed to update applications: {e}"),
                    Ok(()) => {}
                }
            }
            Ok(event) => info!("ignoring {} event", event.r#type),
            Err(e) => warn!("ignoring unreadable event {text:?}: {e}"),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();

    let config = match Config::from_env() {
        Ok(config) => config,
        Err(e) => {
            error!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let mut syncer = match Syncer::new(config) {
        Ok(syncer) => syncer,
        Err(e) => {
            error!("failed to create HTTP client: {e}");
            return ExitCode::FAILURE;
        }
    };

    let mut delay = MIN_RETRY;
    loop {
        match follow_events(&mut syncer).await {
            Ok(greeted) => {
                info!("events connection closed");
                if greeted {
                    delay = MIN_RETRY;
                }
            }
            Err(Error::Unauthorized) => {
                error!("the server rejected the device token; has this device been revoked?");
                delay = MAX_RETRY;
            }
            Err(e) => {
                warn!("events connection failed: {e}");
                // Falls back to polling while the socket is unavailable; cheap thanks to the ETag.
                if let Err(e) = syncer.sync().await {
                    warn!("failed to update applications: {e}");
                }
            }
        }

        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(MAX_RETRY);
    }
}
