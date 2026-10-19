use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use log::{error, info, warn};

use crate::config::Config;

const FALLBACK_ICON: &str = "web-browser";
const FALLBACK_VIDEO_ICON: &str = "video-x-generic";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Web,
    Video,
}

/// One entry from `GET /api/{workspace}/applications`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Application {
    pub id: Box<str>,
    pub kind: Kind,
    pub name: Box<str>,
    pub url: Box<str>,
    /// A server path such as `/static/icons/<hash>.png`.
    pub icon: Option<Box<str>>,
}

/// Ids and icon names come from the server and become file names, so only plain names pass.
pub fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The local file name for an application's icon, if it has a usable one.
pub fn icon_name(application: &Application) -> Option<&str> {
    let name = application
        .icon
        .as_deref()?
        .strip_prefix("/static/icons/")?;
    is_safe_name(name).then_some(name)
}

pub fn applications_dir(config: &Config) -> PathBuf {
    config.data_dir.join("applications")
}

pub fn icons_dir(config: &Config) -> PathBuf {
    config.data_dir.join("icons")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn command(config: &Config, application: &Application) -> String {
    match application.kind {
        Kind::Web => format!(
            "{} --disable-quic --proxy-server={} --password-store=basic --app={}",
            shell_quote(&config.chromium_bin),
            shell_quote(&config.proxy),
            shell_quote(&application.url),
        ),
        Kind::Video => format!(
            "{} --config-dir={} --force-window=immediate --keep-open=no -- {}",
            shell_quote(&config.mpv_bin),
            shell_quote(&config.mpv_config_dir),
            shell_quote(&application.url),
        ),
    }
}

fn desktop_entry(config: &Config, application: &Application, icon: &str) -> String {
    format!(
        "[Desktop Entry]\nName={name}\nExec={launcher} launch {id}\nIcon={icon}\nType=Application\nTerminal=false\n",
        name = application
            .name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        launcher = config.launcher_bin,
        id = application.id,
    )
}

fn start_script(config: &Config, application: &Application) -> String {
    format!(
        "#!{}\nexec {}\n",
        config.shell_bin,
        command(config, application)
    )
}

type Inventory = HashMap<Box<OsStr>, PathBuf>;

fn inventory(dir: &Path) -> std::io::Result<Inventory> {
    Ok(dir
        .read_dir()?
        .flatten()
        .map(|entry| entry.path())
        .filter_map(|path| Some((path.file_name()?.into(), path.clone())))
        .collect())
}

pub fn write_atomically(path: &Path, contents: &[u8], mode: u32) -> std::io::Result<()> {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy();
    let tmp = path.with_file_name(format!(".{file_name}.tmp"));

    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        file.write_all(contents)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }

    result
}

fn remove_all(stale: Inventory) {
    for path in stale.into_values() {
        info!("removing {}", path.display());
        if let Err(e) = std::fs::remove_file(&path) {
            error!("failed to remove {}: {e}", path.display());
        }
    }
}

/// Makes the .desktop files and start scripts match `applications` exactly.
pub fn install(config: &Config, applications: &[Application]) -> std::io::Result<()> {
    let app_dir = applications_dir(config);
    let icons = icons_dir(config);
    std::fs::create_dir_all(&app_dir)?;
    std::fs::create_dir_all(&config.bin_dir)?;

    let mut stale_entries = inventory(&app_dir)?;
    let mut stale_scripts = inventory(&config.bin_dir)?;

    for application in applications {
        if !is_safe_name(&application.id) {
            warn!("skipping application with unusable id {:?}", application.id);
            continue;
        }

        let icon = icon_name(application)
            .map(|name| icons.join(name))
            .filter(|path| path.exists())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| match application.kind {
                Kind::Web => FALLBACK_ICON.into(),
                Kind::Video => FALLBACK_VIDEO_ICON.into(),
            });

        let entry_path = app_dir.join(format!("{}.desktop", application.id));
        let script_path = config.bin_dir.join(application.id.as_ref());
        stale_entries.remove(entry_path.file_name().unwrap_or_default());
        stale_scripts.remove(script_path.file_name().unwrap_or_default());

        write_atomically(
            &entry_path,
            desktop_entry(config, application, &icon).as_bytes(),
            0o644,
        )?;
        write_atomically(
            &script_path,
            start_script(config, application).as_bytes(),
            0o744,
        )?;
    }

    remove_all(stale_entries);
    remove_all(stale_scripts);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn config(home: &Path) -> Config {
        Config {
            server: "https://sieve.example.com".parse().unwrap(),
            workspace: "7e2d99a5-6a8f-4937-9fa6-586629dd6ab5".into(),
            device_token: "token".into(),
            proxy: "https://sieve.example.com:3129".into(),
            chromium_bin: "/nix/store/x-chromium/bin/chromium".into(),
            launcher_bin: "/nix/store/x-launcher/bin/launcher".into(),
            mpv_bin: "/nix/store/x-mpv/bin/mpv".into(),
            mpv_config_dir: "/etc/sieve/mpv".into(),
            shell_bin: "/bin/sh".into(),
            data_dir: home.join(".local/share/sieve/desktop-manager"),
            bin_dir: home.join(".local/bin/sieve/desktop-manager"),
        }
    }

    pub fn temp_home(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sieve-dm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn app(id: &str, kind: Kind, icon: Option<&str>) -> Application {
        Application {
            id: id.into(),
            kind,
            name: "A  Site\nName".into(),
            url: "https://example.com/it's?a=1&b=2".into(),
            icon: icon.map(Into::into),
        }
    }

    #[test]
    fn quotes_urls_for_the_shell() {
        assert_eq!(
            shell_quote("https://youtube.com/watch?v=a&t=1"),
            "'https://youtube.com/watch?v=a&t=1'"
        );
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn only_plain_names_are_safe() {
        for good in ["chromium-domain-12", "video-7", "ab12.png"] {
            assert!(is_safe_name(good), "{good}");
        }
        for bad in ["", "../etc/passwd", "a/b", ".hidden", "a b", "x\n"] {
            assert!(!is_safe_name(bad), "{bad:?}");
        }
        assert_eq!(
            icon_name(&app("a", Kind::Web, Some("/static/icons/ab.png"))),
            Some("ab.png")
        );
        assert_eq!(
            icon_name(&app("a", Kind::Web, Some("/static/icons/../x"))),
            None
        );
        assert_eq!(
            icon_name(&app("a", Kind::Web, Some("https://evil/x.png"))),
            None
        );
        assert_eq!(icon_name(&app("a", Kind::Web, None)), None);
    }

    #[test]
    fn web_apps_go_through_the_proxy_and_videos_through_mpv() {
        let config = config(Path::new("/home/amanda"));
        assert_eq!(
            start_script(&config, &app("chromium-domain-1", Kind::Web, None)),
            "#!/bin/sh\nexec '/nix/store/x-chromium/bin/chromium' --disable-quic \
             --proxy-server='https://sieve.example.com:3129' --password-store=basic \
             --app='https://example.com/it'\\''s?a=1&b=2'\n"
        );
        assert!(
            start_script(&config, &app("video-1", Kind::Video, None)).contains(
                "'/nix/store/x-mpv/bin/mpv' --config-dir='/etc/sieve/mpv' --force-window=immediate"
            )
        );
    }

    #[test]
    fn installs_entries_and_removes_stale_ones() {
        let home = temp_home("install");
        let config = config(&home);
        std::fs::create_dir_all(icons_dir(&config)).unwrap();
        std::fs::write(icons_dir(&config).join("ab.png"), b"png").unwrap();

        let apps = [
            app("chromium-domain-1", Kind::Web, Some("/static/icons/ab.png")),
            app("video-2", Kind::Video, Some("/static/icons/missing.png")),
            app("../escape", Kind::Web, None),
        ];
        install(&config, &apps).unwrap();

        let entry =
            std::fs::read_to_string(applications_dir(&config).join("chromium-domain-1.desktop"))
                .unwrap();
        assert!(entry.contains("Name=A Site Name\n"));
        assert!(
            entry.contains("Exec=/nix/store/x-launcher/bin/launcher launch chromium-domain-1\n")
        );
        assert!(entry.contains(&format!(
            "Icon={}\n",
            icons_dir(&config).join("ab.png").display()
        )));

        let video =
            std::fs::read_to_string(applications_dir(&config).join("video-2.desktop")).unwrap();
        assert!(
            video.contains("Icon=video-x-generic\n"),
            "missing icon falls back"
        );

        let script = config.bin_dir.join("chromium-domain-1");
        assert_eq!(
            std::fs::metadata(&script).unwrap().permissions().mode() & 0o777,
            0o744
        );
        assert!(!home.join(".local/share/sieve/escape.desktop").exists());

        install(&config, &apps[1..2]).unwrap();
        assert!(
            !applications_dir(&config)
                .join("chromium-domain-1.desktop")
                .exists()
        );
        assert!(!script.exists());
        assert!(applications_dir(&config).join("video-2.desktop").exists());

        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn writes_atomically_and_keeps_old_file_on_failure() {
        let dir = temp_home("atomic");
        let path = dir.join("app");
        write_atomically(&path, b"old", 0o744).unwrap();
        write_atomically(&path, b"new", 0o744).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");

        std::fs::create_dir(dir.join(".blocked.tmp")).unwrap();
        let blocked = dir.join("blocked");
        std::fs::write(&blocked, "kept").unwrap();
        assert!(write_atomically(&blocked, b"lost", 0o644).is_err());
        assert_eq!(std::fs::read_to_string(&blocked).unwrap(), "kept");

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
