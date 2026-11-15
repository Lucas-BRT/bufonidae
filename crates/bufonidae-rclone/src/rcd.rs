//! Supervisor and client for `rclone rcd`, rclone's remote-control daemon.
//!
//! One daemon serves every account. It listens on an ephemeral loopback port
//! with random per-session credentials, passed through the environment (never
//! argv, which other local users can read), and never reads the user's
//! `rclone.conf`.

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
#[derive(Debug)]
pub struct Rcd {
    child: Child,
    http: reqwest::Client,
    base_url: String,
    user: String,
    pass: String,
}

impl Rcd {
    /// Starts `rclone rcd` from `binary` and waits until it is serving.
    pub async fn start(binary: impl AsRef<OsStr>) -> Result<Self, Error> {
        let user = random_hex().map_err(Error::Spawn)?;
        let pass = random_hex().map_err(Error::Spawn)?;

        let mut child = Command::new(binary)
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
                seen.join(" | ")
            )))
        })
        .await
        .map_err(|_| Error::Startup("timed out waiting for the daemon".into()))??;

        // Keep draining stderr so the daemon never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });

        Ok(Self {
            child,
            http: reqwest::Client::new(),
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
            let text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<ErrorBody>(&text).map_or(text, |b| b.error);
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

    /// `operations/list`: entries of `remote` (a path inside `fs`).
    pub async fn list(&self, fs: &str, remote: &str) -> Result<Vec<Entry>, Error> {
        let response: ListResponse = self
            .call("operations/list", &json!({ "fs": fs, "remote": remote }))
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

    /// Stops the daemon and waits for it to exit.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        self.child.kill().await.map_err(Error::Spawn)
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
