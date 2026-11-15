# Qualification checklist (live Mega)

The automated tests use fake accounts and rclone's local backend. Nothing in
CI talks to Mega. Before a release, a person runs this checklist against
**throwaway Mega accounts** (never your real ones) with the exact build and
rclone version being released, and records the result below.

Use two accounts, `A` and `B`, with small free quotas. Work in a fresh profile
so nothing of yours is touched:

```
export XDG_CONFIG_HOME=$(mktemp -d) XDG_DATA_HOME=$(mktemp -d)
```

Mark each line pass, fail or skipped, with a note for anything but pass.

## Accounts

- [ ] `account add` with a wrong password fails and stores nothing
      (`account list` is empty, no keyring entry).
- [ ] `account add` with the right password works; the password is in the
      keyring and in no file under `$XDG_CONFIG_HOME` or `$XDG_DATA_HOME`.
- [ ] An account with two-factor authentication: the code is asked, the login
      works, and a second start works without a new code (session reused).
- [ ] The same e-mail added twice (any letter case) is refused.
- [ ] `account reauth` with a new password replaces the old one.
- [ ] Desktop: Accounts shows free space close to what mega.nz shows.

## Storing and getting

- [ ] A small file: `put`, `ls`, `get`, and `sha256sum` of the download equals
      the original.
- [ ] A folder with nested folders, spaces and non-ASCII names: `get` gives an
      identical tree (`diff -r`).
- [ ] A file already stored with the same size is reported as already stored; a
      different file under the same name is reported as DIFFERS and not
      overwritten.
- [ ] Two local files that would get the same stored name are refused before
      anything is uploaded.
- [ ] `get` never replaces an existing local file.

## Copies and recovery

- [ ] `redundancy set mirror:2`, store a file: it is on both A and B on mega.nz.
- [ ] A file that needs two copies with only one account connected is not
      stored, and says why.
- [ ] Delete one copy on mega.nz: `reconcile` reports it, `repair` rebuilds it,
      and a second `reconcile` is clean.
- [ ] Delete every copy on mega.nz: `reconcile --apply` forgets them and `repair`
      reports the file as LOST (not silently gone).
- [ ] Two files with the same name in one Mega folder: `reconcile` reports
      DUPLICATE and changes nothing for that name.
- [ ] The same path with different sizes on A and B (upload by hand): reported as
      a conflict, not imported.

## Failure behaviour

- [ ] Cancel a large upload midway: the file is left unconfirmed, the Files
      screen says so, and Check settles it.
- [ ] `kill -9` the app during a store, start it again: nothing is corrupted,
      Check settles what was pending.
- [ ] With the network off, or one account's password changed on mega.nz: the
      account is reported UNREACHABLE and the other accounts still work.
- [ ] Fill one account nearly to its quota: Accounts flags it almost full, and a
      store that does not fit says so instead of failing halfway.
- [ ] Interrupt a download (kill the network): no partial file at the final
      name, and no `.bufonidae-partial-*` folder is left after the next run.
- [ ] A damaged copy: replace a stored file on mega.nz with different bytes of the
      same size. `get` refuses it (hash mismatch) and uses another copy if any.

## Safety nets

- [ ] Dropping the home folder in the desktop app asks for confirmation.
- [ ] A drop over the large-upload limit asks for confirmation; the limit in
      Settings changes it.
- [ ] The desktop app and the command line cannot open the index together, and
      the message says what to close.
- [ ] `export`, then `import` into a fresh profile, then `account reauth` for each
      account, then `reconcile`: the library matches.
- [ ] A damaged `index.db` fails with a clear error and the file is untouched.

## Packages (on a clean Ubuntu 24.04, no Rust, no rclone installed)

- [ ] `apt install ./bufonidae_*.deb` installs; `bufonidae --version` runs.
- [ ] `bufonidae` uses the shipped rclone, not a system one (`ps` while a command
      runs, or `BUFONIDAE_RCLONE` unset and no `rclone` on PATH).
- [ ] The desktop app starts under Wayland and under X11, with the right icon.
- [ ] The tarball runs from any folder.
- [ ] `sha256sum -c SHA256SUMS` passes.

## Record

| Date | Version | rclone | Tester | Result | Notes |
|------|---------|--------|--------|--------|-------|
|      |         |        |        |        |       |
