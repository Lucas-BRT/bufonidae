//! Supervisor and client for `rclone rcd`, rclone's remote-control daemon.
//!
//! One daemon serves every account. It listens on an ephemeral loopback port
//! with random per-session credentials, passed through the environment (never
//! argv, which other local users can read), and never reads the user's
//! `rclone.conf`.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::process::Stdio;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};

use crate::Error;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(15);
const SERVING_MARKER: &str = "Serving remote control on ";

/// Space information reported by `operations/about`. Fields a backend does not
/// know are `None`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct About {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub free: Option<u64>,
    pub trashed: Option<u64>,
    pub other: Option<u64>,
}

/// Measured network/transfer counters, including retried bytes.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct TransferStats {
    pub bytes: u64,
    pub speed: f64,
}

/// One entry of `operations/list`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Entry {
    pub path: String,
    pub name: String,
    /// `-1` for directories on most backends.
    pub size: i64,
    pub is_dir: bool,
}

#[derive(Deserialize)]
struct ListResponse {
    list: Vec<Entry>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: String,
}

/// A running `rclone rcd`. The process is killed when this value is dropped.
pub struct Rcd {
    child: Child,
    http: reqwest::Client,
    base_url: String,
    user: String,
    pass: String,
}

impl std::fmt::Debug for Rcd {
    // Manual: the derived impl would print the RC credentials.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Rcd")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl Rcd {
    /// Starts `rclone rcd` from `binary` and waits until it is serving.
    pub async fn start(binary: impl AsRef<OsStr>) -> Result<Self, Error> {
        let binary = binary.as_ref();
        verify_binary(binary).await?;
        let user = random_hex().map_err(Error::Spawn)?;
        let pass = random_hex().map_err(Error::Spawn)?;

        let mut command = Command::new(binary);
        // The user's own rclone settings (`RCLONE_CONFIG`, log levels, delete
        // modes, ...) must not change what this daemon does, or leak through
        // its logs. Only what is set below reaches it.
        scrub_rclone_env(&mut command, std::env::vars_os().map(|(k, _)| k));
        let mut child = command
            .args(["rcd", "--rc-addr", "127.0.0.1:0", "--config", ""])
            .env("RCLONE_RC_USER", &user)
            .env("RCLONE_RC_PASS", &pass)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(Error::Spawn)?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| Error::Startup("stderr not captured".into()))?;
        let mut lines = BufReader::new(stderr).lines();

        let mut seen = Vec::new();
        let base_url = tokio::time::timeout(STARTUP_TIMEOUT, async {
            while let Some(line) = lines.next_line().await.map_err(Error::Spawn)? {
                if let Some((_, url)) = line.split_once(SERVING_MARKER) {
                    return Ok(url.trim().trim_end_matches('/').to_owned());
                }
                seen.push(line);
            }
            Err(Error::Startup(format!(
                "process exited before serving; output: {}",
                crate::escape_control(&seen.join(" | "))
            )))
        })
        .await
        .map_err(|_| Error::Startup("timed out waiting for the daemon".into()))??;

        // Keep draining stderr so the daemon never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        Ok(Self {
            child,
            // The daemon is on this machine's loopback: never go through a
            // configured proxy (it would see the credentials) and never follow
            // a redirect (it could point somewhere else).
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(Error::Http)?,
            base_url,
            user,
            pass,
        })
    }

    /// Calls an rc method and deserializes its JSON response.
    pub async fn call<R: DeserializeOwned>(
        &self,
        method: &str,
        params: &impl Serialize,
    ) -> Result<R, Error> {
        let response = self
            .http
            .post(format!("{}/{method}", self.base_url))
            .basic_auth(&self.user, Some(&self.pass))
            .json(params)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            // rclone echoes the request input in its error bodies, and
            // `config/*` inputs carry passwords: never surface those.
            let message = if method.starts_with("config/") {
                "details withheld: the request may contain credentials".to_owned()
            } else {
                let text = response.text().await.unwrap_or_default();
                crate::escape_control(
                    &serde_json::from_str::<ErrorBody>(&text).map_or(text, |b| b.error),
                )
            };
            return Err(Error::Rpc {
                method: method.to_owned(),
                status: status.as_u16(),
                message,
            });
        }
        Ok(response.json().await?)
    }

    /// `operations/about`: space of the remote `fs` (e.g. `mega1:`).
    pub async fn about(&self, fs: &str) -> Result<About, Error> {
        self.call("operations/about", &json!({ "fs": fs })).await
    }

    pub async fn transfer_stats(&self) -> Result<TransferStats, Error> {
        self.call("core/stats", &json!({ "short": true })).await
    }

    pub async fn reset_transfer_stats(&self) -> Result<(), Error> {
        let _: serde_json::Value = self.call("core/stats-reset", &json!({})).await?;
        Ok(())
    }

    /// `operations/list`: entries of `remote` (a path inside `fs`).
    pub async fn list(&self, fs: &str, remote: &str) -> Result<Vec<Entry>, Error> {
        let response: ListResponse = self
            .call("operations/list", &json!({ "fs": fs, "remote": remote }))
            .await?;
        Ok(response.list)
    }

    /// `operations/list` with `recurse` and `filesOnly`: every file under
    /// `remote`, with paths relative to it.
    pub async fn list_files_recursive(&self, fs: &str, remote: &str) -> Result<Vec<Entry>, Error> {
        let response: ListResponse = self
            .call(
                "operations/list",
                &json!({
                    "fs": fs,
                    "remote": remote,
                    "opt": { "recurse": true, "filesOnly": true },
                }),
            )
            .await?;
        Ok(response.list)
    }

    /// `operations/copyfile`: copies one file between (possibly different) remotes.
    pub async fn copy_file(
        &self,
        src_fs: &str,
        src_remote: &str,
        dst_fs: &str,
        dst_remote: &str,
    ) -> Result<(), Error> {
        let _: serde_json::Value = self
            .call(
                "operations/copyfile",
                &json!({
                    "srcFs": src_fs,
                    "srcRemote": src_remote,
                    "dstFs": dst_fs,
                    "dstRemote": dst_remote,
                }),
            )
            .await?;
        Ok(())
    }

    /// `config/create`: defines remote `name` of backend `kind` in the daemon's
    /// in-memory config. Password options are obscured by rclone, as the
    /// backends expect. Nothing is written to disk.
    pub async fn config_create(
        &self,
        name: &str,
        kind: &str,
        parameters: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<(), Error> {
        let _: serde_json::Value = self
            .call(
                "config/create",
                &json!({
                    "name": name,
                    "type": kind,
                    "parameters": parameters,
                    "opt": { "obscure": true, "nonInteractive": true },
                }),
            )
            .await?;
        Ok(())
    }

    /// `config/get`: the stored options of remote `name` (passwords obscured).
    pub async fn config_get(&self, name: &str) -> Result<BTreeMap<String, String>, Error> {
        self.call("config/get", &json!({ "name": name })).await
    }

    /// `config/delete`: forgets remote `name`.
    pub async fn config_delete(&self, name: &str) -> Result<(), Error> {
        let _: serde_json::Value = self.call("config/delete", &json!({ "name": name })).await?;
        Ok(())
    }

    /// Stops the daemon and waits for it to exit.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        self.child.kill().await.map_err(Error::Spawn)
    }

    /// Checks whether the supervised process has exited, reaping it if so.
    pub fn is_running(&mut self) -> Result<bool, Error> {
        self.child
            .try_wait()
            .map(|status| status.is_none())
            .map_err(Error::Spawn)
    }
}

/// Runs `rclone version` and refuses a binary Bufonidae is not tested with.
async fn verify_binary(binary: &OsStr) -> Result<crate::Version, Error> {
    // rclone reads every `RCLONE_*` variable as an option, so the user's own
    // would change (or break) even this check.
    let mut command = Command::new(binary);
    scrub_rclone_env(&mut command, std::env::vars_os().map(|(k, _)| k));
    let output = tokio::time::timeout(
        Duration::from_secs(15),
        command
            .arg("version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| Error::Startup("`rclone version` did not answer".into()))?
    .map_err(Error::Spawn)?;
    let text = String::from_utf8_lossy(&output.stdout);
    let allow = std::env::var_os(crate::ALLOW_UNTESTED_ENV).is_some_and(|v| !v.is_empty());
    crate::check_supported(&text, allow)
}

/// Removes every `RCLONE_*` variable of the current environment from the child.
fn scrub_rclone_env(command: &mut Command, names: impl Iterator<Item = std::ffi::OsString>) {
    for name in names {
        if name.to_string_lossy().starts_with("RCLONE_") {
            command.env_remove(name);
        }
    }
}

fn random_hex() -> std::io::Result<String> {
    use std::fmt::Write as _;
    use std::io::Read as _;

    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn the_users_rclone_settings_are_removed_from_the_child_but_other_variables_are_not() {
        let mut command = Command::new("rclone");
        let names = [
            "RCLONE_CONFIG",
            "RCLONE_CONFIG_PASS",
            "RCLONE_LOG_LEVEL",
            "RCLONE_MEGA_DEBUG",
            "RCLONE_RC_USER",
            "PATH",
            "HTTPS_PROXY",
            "NOT_RCLONE_X",
        ]
        .map(OsString::from);
        scrub_rclone_env(&mut command, names.into_iter());

        let removed: Vec<String> = command
            .as_std()
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for name in [
            "RCLONE_CONFIG",
            "RCLONE_CONFIG_PASS",
            "RCLONE_LOG_LEVEL",
            "RCLONE_MEGA_DEBUG",
            "RCLONE_RC_USER",
        ] {
            assert!(removed.iter().any(|n| n == name), "{name} was not removed");
        }
        // The network settings and PATH still reach rclone (it needs them).
        for name in ["PATH", "HTTPS_PROXY", "NOT_RCLONE_X"] {
            assert!(!removed.iter().any(|n| n == name), "{name} was removed");
        }
    }
}
