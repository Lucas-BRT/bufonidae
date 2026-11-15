use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use bufonidae_core::account::AccountStore;
use bufonidae_core::config::{self, Config};
use bufonidae_core::export;
use bufonidae_core::get;
use bufonidae_core::index::Index;
use bufonidae_core::manager::{self, SharedSecrets};
use bufonidae_core::placement::MostFreeSpace;
use bufonidae_core::put;
use bufonidae_core::reconcile;
use bufonidae_core::repair;
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
    /// Compare the index with what the accounts really hold. Dry run unless
    /// `--apply`; only the local index is ever changed.
    Reconcile {
        /// Only this folder (default: everything on the accounts).
        #[arg(long, default_value = "")]
        prefix: String,
        /// Write the fixes to the index.
        #[arg(long)]
        apply: bool,
    },
    /// Rebuild the missing copies of files that have fewer than wanted
    /// (see `redundancy`), e.g. after an account was lost.
    Repair {
        /// Only this folder (default: everything).
        #[arg(long, default_value = "")]
        prefix: String,
    },
    /// How many copies of each file to keep, like a RAID level.
    #[command(subcommand)]
    Redundancy(RedundancyCommand),
    /// Write a portable backup (accounts without secrets, settings, which
    /// account holds which file) to a JSON file.
    Export {
        file: std::path::PathBuf,
        /// Replace the file if it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Restore a backup made by `export` into an empty profile. Accounts come
    /// back without passwords: sign each in with `account reauth`.
    Import { file: std::path::PathBuf },
    /// Manage connected Mega accounts.
    #[command(subcommand)]
    Account(AccountCommand),
}

#[derive(Subcommand)]
enum RedundancyCommand {
    /// Show the default and the per-folder levels.
    Show,
    /// Set a level: `single`, `mirror` (2 copies) or `mirror:N`. Applies to
    /// files stored from now on; run `repair` to bring existing ones up to it.
    Set {
        level: String,
        /// Only this folder (default: every file without a folder rule).
        #[arg(long)]
        folder: Option<String>,
    },
    /// Remove the rule of a folder.
    Unset { folder: String },
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
    /// Sign an existing account in again, e.g. after a restore or a password
    /// change. Asks for the password on the terminal.
    Reauth { label: String },
    /// Disconnect an account and forget its stored secrets.
    Remove { label: String },
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let path = AccountStore::default_path()?;
    let mut store = AccountStore::load(&path)?;
    let secrets: SharedSecrets = Arc::new(KeyringStore);

    let config_path = Config::default_path()?;
    match cli.command {
        Command::Export { file, force } => {
            if file.exists() && !force {
                bail!(
                    "{} already exists; use --force to replace it",
                    file.display()
                );
            }
            let index = Index::open(&Index::default_path()?).await?;
            let config = Config::load(&config_path)?;
            let export = export::collect(&index, &store, &config).await?;
            export::write(&file, &export)?;
            println!(
                "wrote {} ({} accounts, {} placements, {} lost files). It holds no passwords.",
                file.display(),
                export.accounts.len(),
                export.placements.len(),
                export.lost.len()
            );
        }
        Command::Import { file } => {
            let export = export::read(&file)?;
            let index = Index::open(&Index::default_path()?).await?;
            let mut config = Config::load(&config_path)?;
            export::restore(&export, &index, &mut store, &mut config).await?;
            println!(
                "restored {} accounts and {} placements.",
                export.accounts.len(),
                export.placements.len()
            );
            for account in &export.accounts {
                println!(
                    "  sign in again:  bufonidae account reauth {}",
                    account.label
                );
            }
        }
        Command::Account(AccountCommand::Reauth { label }) => {
            if store.find(&label).is_none() {
                bail!("no account labelled {label:?}");
            }
            let password = rpassword::prompt_password("Mega password: ")?;
            if password.is_empty() {
                bail!("empty password");
            }
            let code = rpassword::prompt_password("2FA code (empty if none): ")?;
            let two_fa = (!code.is_empty()).then_some(code.as_str());
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result =
                manager::reauthorize(&rcd, &store, &secrets, &label, &password, two_fa).await;
            rcd.shutdown().await?;
            let account = result?;
            println!("signed in {} ({})", account.label, account.email);
        }
        Command::Redundancy(cmd) => {
            let mut config = Config::load(&config_path)?;
            match cmd {
                RedundancyCommand::Show => {
                    println!("default  {}", config::describe(config.default_copies()));
                    for (folder, copies) in config.folders() {
                        println!("{}  {}", show(folder), config::describe(*copies));
                    }
                }
                RedundancyCommand::Set { level, folder } => {
                    let copies = config::parse_level(&level)?;
                    config.set(folder.as_deref(), copies)?;
                    println!("ok: {}", config::describe(copies));
                }
                RedundancyCommand::Unset { folder } => {
                    config.unset(&folder)?;
                    println!("removed the rule of {}", show(&folder));
                }
            }
        }
        Command::Repair { prefix } => {
            let config = Config::load(&config_path)?;
            let index = Index::open(&Index::default_path()?).await?;
            let accounts = store.list().to_vec();
            if accounts.is_empty() {
                bail!("no accounts connected; run `bufonidae account add` first");
            }
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                for (account, result) in manager::connect_all(&rcd, &store, &secrets).await {
                    if let Err(e) = result {
                        eprintln!("warning: account {:?}: {e}", account.label);
                    }
                }
                Ok::<_, anyhow::Error>(
                    repair::repair(
                        &rcd,
                        &index,
                        &accounts,
                        &MostFreeSpace::default(),
                        &config,
                        &prefix,
                    )
                    .await?,
                )
            }
            .await;
            rcd.shutdown().await?;
            let report = result?;
            for (p, label) in &report.repaired {
                let p = show(p);
                let label = show(label);
                println!("copied      {p}  ->  {label}");
            }
            for (label, why) in &report.unreachable {
                let label = show(label);
                let why = show(why);
                eprintln!(
                    "UNREACHABLE {label}: left out, and the copies it holds were not counted: {why}"
                );
            }
            for p in &report.lost {
                let p = show(p);
                eprintln!("LOST        {p}: no copy on any connected account");
            }
            for (p, n) in &report.no_room {
                let p = show(p);
                eprintln!("NO ROOM     {p}: {n} copies could not be placed");
            }
            for (p, reason) in &report.failed {
                let p = show(p);
                let reason = show(reason);
                eprintln!("FAILED      {p}: {reason}");
            }
            if !report.is_complete() {
                std::process::exit(1);
            }
        }
        Command::Reconcile { prefix, apply } => {
            let index = Index::open(&Index::default_path()?).await?;
            let accounts = store.list().to_vec();
            if accounts.is_empty() {
                bail!("no accounts connected; run `bufonidae account add` first");
            }
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                for (account, result) in manager::connect_all(&rcd, &store, &secrets).await {
                    if let Err(e) = result {
                        eprintln!("warning: account {:?}: {e}", account.label);
                    }
                }
                Ok::<_, anyhow::Error>(
                    reconcile::reconcile(&rcd, &index, &accounts, &prefix, apply).await?,
                )
            }
            .await;
            rcd.shutdown().await?;
            let report = result?;
            print_reconcile(&report);
            let config = Config::load(&config_path)?;
            let gaps = repair::find_gaps(&index, &accounts, &config, &prefix).await?;
            for g in &gaps {
                println!(
                    "UNDER-COPIED  {}: {} of {} copies; run `bufonidae repair`",
                    g.path,
                    g.holders.len(),
                    g.wanted
                );
            }
            if report.unresolved() > 0 || (!apply && report.fixable() > 0) || !gaps.is_empty() {
                std::process::exit(1);
            }
        }
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
                println!(
                    "{:>12}  {}  {}",
                    file.size,
                    show(&labels.join(",")),
                    show(&file.path)
                );
            }
        }
        Command::Get { path, out } => {
            let index = Index::open(&Index::default_path()?).await?;
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                let usable = sign_in(&rcd, &store, &secrets).await;
                if usable.is_empty() {
                    bail!("no account could sign in");
                }
                Ok::<_, anyhow::Error>(get::get(&rcd, &index, &usable, &path, &out).await?)
            }
            .await;
            rcd.shutdown().await?;
            let report = result?;
            for (p, label, size) in &report.downloaded {
                let p = show(p);
                let label = show(label);
                println!("downloaded  {p}  <-  {label}  ({size} bytes)");
            }
            for p in &report.already_there {
                let p = show(&p.display().to_string());
                println!("exists      {p}");
            }
            for (label, why) in &report.unreachable {
                let label = show(label);
                let why = show(why);
                eprintln!("UNREACHABLE {label}: not used: {why}");
            }
            for (p, reason) in &report.failed {
                let p = show(p);
                let reason = show(reason);
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
            let config = Config::load(&config_path)?;
            let index = Index::open(&Index::default_path()?).await?;
            let rcd = Rcd::start("rclone")
                .await
                .context("could not start rclone; is it installed and on PATH?")?;
            let result = async {
                let signed_in = sign_in(&rcd, &store, &secrets).await;
                let usable: Vec<_> = chosen
                    .iter()
                    .filter(|c| signed_in.iter().any(|s| s.id == c.id))
                    .cloned()
                    .collect();
                if usable.is_empty() {
                    bail!("none of the chosen accounts could sign in");
                }
                Ok::<_, anyhow::Error>(
                    put::put(
                        &rcd,
                        &index,
                        &usable,
                        &MostFreeSpace::default(),
                        &config,
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
        let rel = show(rel);
        let label = show(label);
        println!("uploaded  {rel}  ->  {label}  ({size} bytes)");
    }
    for rel in &report.already_there {
        let rel = show(rel);
        println!("exists    {rel}");
    }
    for path in &report.skipped {
        let path = show(&path.display().to_string());
        println!("skipped   {path} (symlink or unsupported name)");
    }
    for (label, why) in &report.unreachable {
        let label = show(label);
        let why = show(why);
        eprintln!("UNREACHABLE {label}: not used: {why}");
    }
    for (rel, reason) in &report.differs {
        let rel = show(rel);
        let reason = show(reason);
        eprintln!("DIFFERS   {rel}: {reason}");
    }
    for rel in &report.no_room {
        let rel = show(rel);
        eprintln!("NO ROOM   {rel} (not enough accounts or space for every copy)");
    }
    for (rel, reason) in &report.failed {
        let rel = show(rel);
        let reason = show(reason);
        eprintln!("FAILED    {rel}: {reason}");
    }
}

fn print_reconcile(r: &reconcile::Report) {
    let verb =
        |applied: bool, done: &'static str, would: &'static str| if applied { done } else { would };
    println!("consistent  {}", r.consistent);
    for (p, a) in &r.confirmed {
        let p = show(p);
        let a = show(a);
        println!(
            "{}  {p}  ({a})",
            verb(r.applied, "confirmed  ", "would confirm")
        );
    }
    for (p, a) in &r.dropped {
        let p = show(p);
        let a = show(a);
        println!(
            "{}  {p}  ({a}): not on the remote",
            verb(r.applied, "forgot     ", "would forget")
        );
    }
    for (p, a, size) in &r.imported {
        let p = show(p);
        let a = show(a);
        println!(
            "{}  {p}  ({a}, {size} bytes)",
            verb(r.applied, "imported   ", "would import")
        );
    }
    for (p, a, want, got) in &r.size_mismatch {
        let p = show(p);
        let a = show(a);
        eprintln!("SIZE       {p} ({a}): index says {want}, remote has {got}");
    }
    for (p, a, want, got) in &r.partial {
        let p = show(p);
        let a = show(a);
        eprintln!("PARTIAL    {p} ({a}): expected {want}, remote has {got}; left in place");
    }
    for (p, a, want, got) in &r.conflicts {
        let p = show(p);
        let a = show(a);
        eprintln!("CONFLICT   {p} ({a}): other copies are {want} bytes, this one is {got}");
    }
    for (p, a) in &r.duplicates {
        let p = show(p);
        let a = show(a);
        eprintln!(
            "DUPLICATE  {p} ({a}): the name exists twice there, so it was left alone; \
             remove one of them on mega.nz"
        );
    }
    for (a, e) in &r.unreachable {
        let a = show(a);
        let e = show(e);
        eprintln!("UNREACHABLE {a}: {e}; its entries were not touched");
    }
    if r.unverifiable > 0 {
        println!(
            "{} index entries are on accounts that are not connected",
            r.unverifiable
        );
    }
    if !r.applied && r.fixable() > 0 {
        println!("dry run: nothing written; run again with --apply");
    }
}

/// Signs every stored account in and returns those that worked; the others
/// are warned about on stderr and left out, so one broken account does not
/// stop work on the healthy ones.
async fn sign_in(
    rcd: &Rcd,
    store: &AccountStore,
    secrets: &manager::SharedSecrets,
) -> Vec<bufonidae_core::account::Account> {
    let mut usable = Vec::new();
    for (account, result) in manager::connect_all(rcd, store, secrets).await {
        match result {
            Ok(()) => usable.push(account),
            Err(e) => eprintln!(
                "warning: account {:?} could not sign in and will not be used: {e}",
                account.label
            ),
        }
    }
    usable
}

/// Makes a name safe to print: control characters (which could redraw or
/// rewrite the terminal) are shown as `\u{..}`. File names and rclone's
/// messages are untrusted.
fn show(text: &str) -> String {
    bufonidae_rclone::escape_control(text)
}
