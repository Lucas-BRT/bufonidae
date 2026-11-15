# Backup and recovery

Bufonidae keeps a local **index**: which account holds which file, the
redundancy settings, and which files were lost. A remote listing cannot recreate
that, so treat the index as data worth backing up, not as a disposable cache.

## Back up

```
bufonidae export backup.json
```

The file is plain JSON (accounts without passwords, settings, placements, lost
files). It is written privately (mode 0600) and atomically, and the command
refuses to replace an existing file unless you pass `--force`. It contains **no
password, session or master key**, but it does list your account e-mail
addresses and folder structure, so store it accordingly.

Export while the desktop app is closed: only one process can open the index.

## Restore into a clean profile

```
bufonidae import backup.json
bufonidae account reauth <label>      # once per account, asks for the password
bufonidae reconcile                   # compare with what the accounts really hold
```

`import` only works on an empty index and an empty account list, and checks the
whole file before changing anything. Accounts come back with their old ids, so
the restored index still points at them.

## If the index file is damaged

Opening it fails with an error and the file is **left untouched**. Do not
delete it. Move it aside (`~/.local/share/bufonidae/index.db*`, including the
`-wal` file), then either restore a backup as above, or start empty and run
`bufonidae reconcile --apply` to rebuild what the accounts hold. Rebuilding from
the accounts alone loses: redundancy rules (kept in `config.toml`, not the
index), the record of lost files, and any file whose copies disagree.

## What is not built

- There is no automatic scheduled backup.
- There is no importer into another database. The JSON export is the portable
  format; if Turso (a beta database) ever has to be replaced, the export is how
  data moves. A tested replacement path is a release task, not a feature today.
- Content hashes are not stored, so a restore cannot prove file contents.
