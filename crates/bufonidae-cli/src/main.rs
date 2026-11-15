use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bufonidae_core::account::AccountStore;
use bufonidae_core::manager::{self, SharedSecrets};
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
