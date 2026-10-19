use std::path::PathBuf;

/// Everything comes from the environment the NixOS module gives the user service.
pub struct Config {
    /// The Sieve server's base URL, e.g. `https://sieve.example.com`.
    pub server: url::Url,
    pub workspace: Box<str>,
    pub device_token: Box<str>,
    /// Passed to Chromium as `--proxy-server`, e.g. `https://sieve.example.com:3129`.
    pub proxy: Box<str>,
    pub chromium_bin: Box<str>,
    pub launcher_bin: Box<str>,
    pub mpv_bin: Box<str>,
    pub mpv_config_dir: Box<str>,
    pub shell_bin: Box<str>,
    pub data_dir: PathBuf,
    pub bin_dir: PathBuf,
}

fn var(name: &str) -> Result<Box<str>, String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value.trim().into()),
        _ => Err(format!("{name} is not set")),
    }
}

fn var_or(name: &str, default: &str) -> Box<str> {
    var(name).unwrap_or_else(|_| default.into())
}

impl Config {
    pub fn from_env() -> Result<Self, String> {
        let server: url::Url = var("SIEVE_SERVER")?
            .parse()
            .map_err(|e| format!("SIEVE_SERVER is not a URL: {e}"))?;
        if !matches!(server.scheme(), "https" | "http") {
            return Err("SIEVE_SERVER must be an http(s) URL".into());
        }

        let token_file = var("SIEVE_DEVICE_TOKEN_FILE")?;
        let device_token = std::fs::read_to_string(token_file.as_ref())
            .map_err(|e| format!("failed to read {token_file}: {e}"))?
            .trim()
            .into();

        let home: PathBuf = var("HOME")?.as_ref().into();

        Ok(Self {
            server,
            workspace: var("SIEVE_WORKSPACE")?,
            device_token,
            proxy: var("SIEVE_PROXY")?,
            chromium_bin: var_or("CHROMIUM_BIN", "chromium"),
            launcher_bin: var_or("LAUNCHER_BIN", "launcher"),
            mpv_bin: var_or("MPV_BIN", "mpv"),
            mpv_config_dir: var_or("MPV_CONFIG_DIR", "/etc/sieve/mpv"),
            shell_bin: var_or("SHELL_BIN", "/bin/sh"),
            // The launcher reads start scripts from here, and the module adds the data dir to
            // XDG_DATA_DIRS so the drawer finds the .desktop files.
            data_dir: home.join(".local/share/sieve/desktop-manager"),
            bin_dir: home.join(".local/bin/sieve/desktop-manager"),
        })
    }

    pub fn api_url(&self, path: &str) -> url::Url {
        let mut url = self.server.clone();
        url.set_path(&format!("/api/{}{path}", self.workspace));
        url
    }

    pub fn events_url(&self) -> url::Url {
        let mut url = self.api_url("/events");
        let scheme = if self.server.scheme() == "https" { "wss" } else { "ws" };
        url.set_scheme(scheme).expect("ws and wss are valid schemes for an http(s) URL");
        url
    }

    pub fn server_url(&self, path: &str) -> url::Url {
        let mut url = self.server.clone();
        url.set_path(path);
        url
    }
}
