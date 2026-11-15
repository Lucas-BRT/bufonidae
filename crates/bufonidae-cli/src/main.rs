use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bufonidae_core::account::AccountStore;
use bufonidae_core::get;
use bufonidae_core::index::Index;
use bufonidae_core::manager::{self, SharedSecrets};
use bufonidae_core::placement::MostFreeSpace;
use bufonidae_core::put;
use bufonidae_core::secrets::KeyringStore;
use bufonidae_rclone::rcd::Rcd;
use clap::{Parser, Subcommand};

/// Pool several Mega.nz accounts into one logical directory, via rclone.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Copy files or folders into the pool, spreading files across accounts.
    Put {
        /// Files and folders to upload.
        #[arg(required = true)]
        paths: Vec<std::path::PathBuf>,
        /// Destination folder inside the accounts, e.g. `Photos/2026`.
        #[arg(long)]
        to: String,
        /// Accounts to use (default: all connected accounts).
        #[arg(long, value_delimiter = ',')]
        accounts: Vec<String>,
    },
    /// List stored files (from the local index, no network needed).
    Ls {
        /// Only files under this folder.
        #[arg(default_value = "")]
        prefix: String,
    },
    /// Download a file or folder from the pool.
    Get {
        /// File or folder, as shown by `ls`.
        path: String,
        /// Local destination folder.
        #[arg(long, default_value = ".")]
        out: std::path::PathBuf,
    },
    /// Manage connected Mega accounts.
    #[command(subcommand)]
    Account(AccountCommand),
}

#[derive(Subcommand)]
enum AccountCommand {
    /// Connect an account. The password (and 2FA code) are asked on the
    /// terminal, never taken from arguments.
    Add {
        /// Short unique name, e.g. `work`.
        label: String,
        /// Mega account e-mail.
        #[arg(long)]
        email: String,
    },
    /// List connected accounts.
    List,
    /// Disconnect an account and forget its stored secrets.
    Remove { label: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = AccountStore::default_path()?;
    let mut store = AccountStore::load(&path)?;
    let secrets: SharedSecrets = Arc::new(KeyringStore);

    match cli.command {
        Command::Ls { prefix } => {
            let index = Index::open(&Index::default_path()?).await?;
            let prefix = prefix.trim_matches('/');
            for file in index.list(prefix).await? {
                if !(prefix.is_empty()
                    || file.path == prefix
                    || file.path.starts_with(&format!("{prefix}/")))
                {
                    continue;
                }
                let labels: Vec<&str> = file
                    .accounts
                    .iter()
                    .map(|id| {
                        store
                            .list()
                            .iter()
                            .find(|a| &a.id == id)
                            .map_or("?", |a| a.label.as_str())
                    })
                    .collect();
                println!("{:>12}  {}  {}", file.size, labels.join(","), file.path);
            }
        }
        Command::Get { path, out } => {
            let index = Index::open(&Index::default_path()?).await?;
            let accounts = store.list().to_vec();
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                for (account, result) in manager::connect_all(&rcd, &store, &secrets).await {
                    if let Err(e) = result {
                        eprintln!("warning: account {:?}: {e}", account.label);
                    }
                }
                Ok::<_, anyhow::Error>(get::get(&rcd, &index, &accounts, &path, &out).await?)
            }
            .await;
            rcd.shutdown().await?;
            let report = result?;
            for (p, label, size) in &report.downloaded {
                println!("downloaded  {p}  <-  {label}  ({size} bytes)");
            }
            for p in &report.already_there {
                println!("exists      {}", p.display());
            }
            for (p, reason) in &report.failed {
                eprintln!("FAILED      {p}: {reason}");
            }
            if !report.is_complete() {
                std::process::exit(1);
            }
        }
        Command::Put {
            paths,
            to,
            accounts,
        } => {
            let chosen: Vec<_> = if accounts.is_empty() {
                store.list().to_vec()
            } else {
                accounts
                    .iter()
                    .map(|l| {
                        store
                            .find(l)
                            .cloned()
                            .with_context(|| format!("no account labelled {l:?}"))
                    })
                    .collect::<Result<_>>()?
            };
            if chosen.is_empty() {
                bail!("no accounts connected; run `bufonidae account add` first");
            }
            let index = Index::open(&Index::default_path()?).await?;
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                for (account, result) in manager::connect_all(&rcd, &store, &secrets).await {
                    if chosen.iter().any(|c| c.id == account.id) {
                        result.with_context(|| format!("account {:?}", account.label))?;
                    }
                }
                Ok::<_, anyhow::Error>(
                    put::put(
                        &rcd,
                        &index,
                        &chosen,
                        &MostFreeSpace::default(),
                        &paths,
                        &to,
                    )
                    .await?,
                )
            }
            .await;
            rcd.shutdown().await?;
            let report = result?;
            print_report(&report);
            if !report.is_complete() {
                std::process::exit(1);
            }
        }
        Command::Account(AccountCommand::List) => {
            for a in store.list() {
                println!("{}\t{}", a.label, a.email);
            }
        }
        Command::Account(AccountCommand::Remove { label }) => {
            manager::remove_account(&mut store, &secrets, &label).await?;
            println!("removed {label}");
        }
        Command::Account(AccountCommand::Add { label, email }) => {
            // Validate the label before asking for secrets.
            store.prepare(&label, &email)?;
            let password = rpassword::prompt_password("Mega password: ")?;
            if password.is_empty() {
                bail!("empty password");
            }
            let code = rpassword::prompt_password("2FA code (empty if none): ")?;
            let two_fa = (!code.is_empty()).then_some(code.as_str());

            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = manager::add_account(
                &rcd, &mut store, &secrets, &label, &email, &password, two_fa,
            )
            .await;
            rcd.shutdown().await?;
            let account = result?;
            println!("connected {} ({})", account.label, account.email);
        }
    }
    Ok(())
}

fn print_report(report: &put::Report) {
    for (rel, label, size) in &report.uploaded {
        println!("uploaded  {rel}  ->  {label}  ({size} bytes)");
    }
    for rel in &report.already_there {
        println!("exists    {rel}");
    }
    for path in &report.skipped {
        println!("skipped   {} (symlink or unsupported name)", path.display());
    }
    for rel in &report.no_room {
        eprintln!("NO ROOM   {rel}");
    }
    for (rel, reason) in &report.failed {
        eprintln!("FAILED    {rel}: {reason}");
    }
}
