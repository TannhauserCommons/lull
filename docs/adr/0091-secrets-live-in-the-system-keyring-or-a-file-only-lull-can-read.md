# ADR-0091: A listener's secrets live in the system's keyring, or in a file only lull can read

- Status: accepted (by the owner, 2026-11-07)
- Date: 2026-11-07

## In plain words

- **Today** lull keeps the listener's passwords and keys as plain text in its database. That covers Soulseek, Jellyfin, ListenBrainz, AcoustID, the Last.fm sign-in and the apps' own pairing tokens. The Soulseek password also sits in a small file for slskd. Anyone who can read those files, or a copy of them, can read the passwords. A copy can be a backup, a database lull saves before an update, or a file sent with a bug report.
- **On a computer running the lull app,** lull will keep them in the system's own password store: Windows Credential Manager, or GNOME Keyring or KWallet on Linux. The database only remembers "this one is in the keyring".
- **On a server** (the Linux package, Docker) there is no such store. There lull keeps them in one file that only lull's own user can open. They are never in the database or its backups.
- **A password the admin set** in the environment or a `.env` file stays there. lull never copies it.
- **The Soulseek password leaves slskd's file.** lull hands it to slskd when it starts slskd, the way it already hands over slskd's own key.
- **Settings says where each password is kept**, in one plain sentence.
- **Upgrading moves them by itself.** The old copy is erased only after the new place has confirmed it holds the password.
- **If the keyring is locked or gone later,** lull says so and asks. It never drops a login silently.
- **On the phone,** the token that pairs it with the hub moves into Android's Keystore-protected storage and is left out of phone backups.
- **What this doesn't protect against:** a program already running as you can ask your keyring too, on every system. The keyring protects copies of files, other user accounts on the computer, and a disk read while you're logged out. That is what any app can honestly promise.

## Context

### What is secret, and where it is today (origin/main, read in the code)

| Credential | Where it's kept today | Notes |
|---|---|---|
| Every `Key::secret` setting (`settings::KEYS`): the Soulseek password, the slskd API key and web password, the Jellyfin key, the Last.fm API key and secret, the AcoustID keys, the ListenBrainz token | `core_config` in `lull.db`, plain | The UI only learns whether one is set. Keep that. |
| Last.fm session key, after sign-in | `lastfm_session.session_key` in `lull.db`, plain | Owned by the lastfm crate |
| Subsonic app passwords | `core_devices.secret`, plain | ADR-0002 accepted this: Subsonic's `md5(password + salt)` needs the password itself |
| The Soulseek username and password, a second copy | `slskd.yml` (`soulseek/src/account.rs`), written with `tokio::fs::write` | The file mode comes from the umask, typically 0644 on Linux. The slskd folder is made with `create_dir_all`, typically 0755. |
| The apps' device token | `settings.json` in the app's config folder (`app/src/store.rs`), on the desktop and on Android | The Android manifest doesn't set `allowBackup`, so Android's auto-backup copies the file to the phone's cloud backup |
| Copies made before an update | `<data>/backups/before-<version>.db` (`server/src/updates.rs`) | Whole copies of the database, with every secret above. Kept after the update. |
| Paired browsers and apps on the hub | `core_devices.token_hash`: sha256 only | Fine as it is |
| The managed slskd's API key and web password | random each start, passed in the child's environment | Fine as it is (`server/src/managed/slskd.rs`) |

Two more leaks:
- **`lull config set KEY VALUE`** takes a secret on the command line (`server/src/cli/mod.rs`). On Linux every user can read it in `ps` while it runs, and it stays in the shell's history.
- **"Deleted" isn't gone in SQLite.** A deleted row's bytes stay in the file's free pages and in the WAL until they are overwritten. Removing a plain copy therefore needs `secure_delete` and a checkpoint.

### Where the hub runs, and as whom

- **The desktop app** runs the hub in its own process, as the listener, inside their login session. Their keyring is reachable: Credential Manager on Windows, the Secret Service on the session D-Bus on Linux.
- **The Linux package** runs as the system user `lull`. It has no login session and no session D-Bus, so no Secret Service. Its data is in `/var/lib/lull` (`StateDirectoryMode=0750`). There is no `/etc/lull/lull.conf`: ADR-0090 rejected a settings file, and settings live in the database.
- **Docker** runs as distroless `nonroot`, with data in the volume. The admin's settings come from the environment, or from the `.env` or `LULL_CONFIG` file.
- **Android** is a client only (ADR-0006). Its one secret is its device token.
- **macOS** isn't built (ADR-0090).

### What a system keyring protects

- **Windows:** Credential Manager encrypts each credential with DPAPI under the user's login. Other users can't read it, and neither can a copied disk without the user's password. Any process running as that user can read it.
- **Linux:** the Secret Service (GNOME Keyring, KWallet, KeePassXC) encrypts its collection with the login password and unlocks it at login. Once unlocked, any process of that user on the session bus can read it.
- **Android:** a Keystore key, held by the system or by hardware, encrypts the app's stored values. Only the app can use the key.

So the keyring protects copies of files and other accounts, not the user from their own programs.

### Downgrades

`Db::migrate` refuses a database whose owner is further along than the build knows. A new step under `core` or `lastfm` would therefore stop every older lull from starting. A new migrations owner is invisible to older builds, as the `identity` owner was (ADR-0035).

### The library

**keyring-rs, version 4:** `keyring-core` 1.0, plus one crate per store, all from the same project:
- `windows-native-keyring-store`;
- `zbus-secret-service-keyring-store` and `dbus-secret-service-keyring-store`;
- `android-native-keyring-store`;
- `apple-native-keyring-store`.

The app already links zbus 5 on Linux, so the zbus store adds no D-Bus stack.

## Decision

### 1. One rule

No credential is written to lull's database, to its backups, or to any file other than the secrets file below. A credential is:
- every `Key::secret`;
- the Last.fm session key;
- the Subsonic app passwords;
- the apps' device tokens.

### 2. One seam, chosen by whoever runs the hub

**`lull_core::secrets`** holds the hub's credentials in memory. It reads them once at start and writes changes through to a **vault**. There are three vaults:

- **`Keyring`:** the system's store, through `keyring-core`. It lives in `app/` only, so the Linux package and the image carry no keyring code.
  - Each credential is one entry. Its service is the app's identifier, so a debug build with its own identifier never touches the installed app's entries.
  - Its name is the key plus a short hash of the data folder, so two hubs on one account stay apart.
  - It has a readable label, such as "lull: Soulseek password", because that is what the listener sees in Credential Manager or Seahorse.
- **`File`:** `<data>/secrets/secrets.json`.
  - The folder is created 0700 and the file 0600, with the mode given at creation (`DirBuilder::mode`, `OpenOptions::mode`). Nothing is chmod'ed afterwards.
  - A change is written to a new file in the same folder, also created 0600, and renamed over the old one.
  - Once it exists, lull checks the modes at start. If they are wider, lull refuses to read it and says why.
  - On Windows the file inherits the data folder's ACL. That folder is under the user's profile, which only that user, SYSTEM and Administrators can open.
- **`Env`:** read-only. It holds what the admin set in the environment or in `LULL_CONFIG`.

**Which vault is used:**
- **The desktop app** proves the keyring at start: it writes, reads back and deletes a test entry.
  - If that works, the app uses the keyring.
  - If no keyring service exists at all, the app uses the file. That happens on Linux without a Secret Service, for example a bare window manager.
- **The Linux package, Docker and `mise run dev`** use the file.
- **The admin's environment** always wins, as it does now (ADR-0002).

**Where each credential is kept** is recorded in a new table, `secrets_kept (key, vault, updated_at)`, under a new migrations owner `secrets`. Older builds never look at that owner.

### 3. When the keyring is missing, locked or refuses

- **No keyring on the machine:** the app uses the file. Settings says so, and says a keyring (GNOME Keyring or KWallet) would keep the passwords there instead. When one appears later, the next start moves them over, with the same confirm-then-erase rule as in section 5.
- **The keyring was in use and now fails:**
  - This covers a locked keyring, an unlock prompt the listener dismissed, KWallet refusing, and an entry that has gone.
  - lull does not fall back. A fallback would lower the protection silently, and lull can't read the value to move it anyway.
  - The credential stays recorded where it was. The parts that need it report it in `/health` and the status tray, for example: "lull can't open your keyring (it's locked). Unlock it and press Try again, or keep your passwords in lull's own file instead."
  - The second choice asks the listener to type the password again. Nothing is ever erased.
- **The environment and `LULL_CONFIG`:** they stay where the admin put them.
  - If `LULL_CONFIG` holds a secret and others can read the file (Unix group or other bits), lull warns at start, in its log and in Settings. It never changes that file's mode itself.
  - `deploy/` documents `chmod 600 .env`.
- **The Linux package:**
  - `lull setup` and the web page send credentials to the hub as now, and they land in the file vault.
  - The data folder stays 0750 (ADR-0090, delivery's). The `secrets/` folder inside it is 0700, so the `lull` group can't read it.
- **`lull config set`** never takes a secret from the command line. `lull config set KEY` with no value prompts without echoing, as `lull setup` already does. `-` reads the value from stdin, for scripts. A secret given as an argument is refused, with the reason.

### 4. slskd's Soulseek login

The managed slskd gets the account in its own environment: `SLSKD_SLSK_USERNAME` and `SLSKD_SLSK_PASSWORD`, slskd's documented options. Its API key already travels this way.

- **Who can read it:**
  - On Linux, a child's environment is readable through `/proc/<pid>/environ` (mode 0400) by the same user and root only.
  - On Windows, it is readable by the same user and administrators.
  - Those are the same people who can read a 0600 file, but nothing is written to disk.
- **The cost:** a changed account restarts slskd, rather than slskd reloading its file. The supervisor already restarts slskd when the shares change. A new account drops the Soulseek session either way, so running transfers end in both cases. This also removes `account.rs`'s 60-second wait for a reload.
- **`slskd.yml` stays, empty:**
  - slskd reads its yml over its environment, so a stale yml from an older version would win. lull therefore keeps the file, empty and its own.
  - The slskd folder is created 0700 and the file 0600.
  - The first start of the new version rewrites an existing file without the account.
- **An slskd the admin runs** (the compose kit, an existing slskd) keeps its own login. That stays the admin's.
- **To verify** in `slskd-contract.yml`: that slskd doesn't write the password anywhere else in its app folder.

### 5. Moving what's there (upgrade)

At hub start, before any part reads its settings:

1. Find every plain credential: secret rows in `core_config`, `core_devices.secret`, and `lastfm_session.session_key`. The last one is moved by the lastfm crate itself on its start, through the seam.
2. Write each one to the vault, read it back, and compare.
3. Only then, in one transaction with `PRAGMA secure_delete = ON`:
   - remove the plain copy;
   - record it in `secrets_kept`.

   Then checkpoint the WAL (`TRUNCATE`).
4. Anything fails: the plain copy stays, the log names the key (never its value), and the next start tries again.

An ordinary update never asks anyone to type a password again, on any system: the move needs no answer from the listener. A keyring that asks to be unlocked at that moment is the system asking, not lull, and saying no leaves the plain copy for the next start.

Later states:
- **A plain row found later** was written by an older lull after a downgrade. It is newer than the vault's copy, so it is moved again over it.
- **Backups (`backups/before-*.db`):**
  - Once the new version is past its health window (ADR-0090), lull removes the credentials from those copies, with `secure_delete` and `VACUUM`.
  - New backups are created 0600.
  - Going back with a cleaned copy gives an older lull that asks for the passwords again. Inside the health window the copy is untouched, so a failed update rolls back with every login intact.
- **The apps' `settings.json` token:** it moves to the keyring (desktop) or the Keystore-backed store (Android) the same way. It is removed from the file only after the store confirms it.
- **Android:**
  - `settings.json` is excluded from backup and device transfer (`dataExtractionRules` and `fullBackupContent`).
  - This lands first, on its own.

### 6. Going back to an older lull

There is no step under `core`, `lastfm` or any other existing owner, so an older lull still starts:
- **Secrets that were moved:** it finds no plain row, treats them as unset, and Settings asks for them. It doesn't crash.
- **The Last.fm sign-in:** after the move it shows as signed out.
  - The exact form of the emptied row is picked by testing the release before this one against it. An empty `session_key` must read as "not connected", not as repeated errors.
  - If it doesn't, the row is removed. In that case the two Last.fm switches go back to their defaults.
- **Subsonic apps:** they need a new app password.
- **The desktop app** makes itself a new device token for its own hub. Against a remote hub, it pairs again.
- **Upgrading again** finds the moved passwords still in the vault. Anything the older lull asked for and stored in plain is newer, so it moves over them.

### 7. What Settings says

`GET /api/config` adds a `kept` field to each secret: `keyring`, `file`, `env` or `config-file`. It also adds the sentence Settings shows under the field:

| `kept` | Settings shows |
|---|---|
| `keyring` | "Kept in Windows Credential Manager." or "Kept in your system keyring (Secret Service)." |
| `file` | "Kept in lull's own file, which only lull's account can open: `<path>`." |
| `env` | "Set in the server's environment." This is the existing locked state. |
| `config-file` | "Set in `<path>`." |

The value itself still never reaches the UI. The Settings forms are lull-graphics' while it reworks them, so this is one shared line those forms render. Settings isn't on the start-up path, so the start-up bytes don't change (to be checked against the budget).

### 8. Never logged, never echoed

- `Secret` already prints `***`.
- Vault errors name the key, never the value.
- A config error about a secret never quotes the value.
- **One test pins it.** A hub is given a distinct canary value for every credential, and its logs are captured at TRACE. It then goes through:
  - setting each credential;
  - the move;
  - starting slskd (the fake);
  - connecting Last.fm (the fake);
  - switching to Jellyfin;
  - a downgrade and upgrade.

  The test asserts that no canary appears in any log line, in any API response (`/api/config`, `/health`, the event stream), or in any file under the data folder except the file vault's.
- **The hub contract** (the package and the image) does the same on the journal and the container log, for its own canary.

### 9. Proof in CI

| Where | What it proves |
|---|---|
| Linux with a Secret Service: `dbus-run-session`, then `gnome-keyring-daemon --unlock` with an empty password | The keyring vault round trip; the move; a keyring locked through D-Bus `Lock` gives a status and keeps the data |
| Linux without one: no session bus | The file vault. The test runs under `umask 000` to prove the modes come from creation, not from the umask. |
| Windows (`windows-latest`) | The Credential Manager round trip and the move |
| Android | The build compiles it. The Keystore needs a device: an emulator job if CI gets one. Until then it is checked by hand on a phone, and reported as not verified in CI. |
| Upgrade, every OS the CI runs | A hub made by 0.2.6 with saved logins starts on the new version: every login works with nothing typed. A rollback inside the health window comes back with every login. |
| The server package contract (delivery's) | After `lull setup`, `/var/lib/lull/secrets` is 0700 `lull:lull` and the file is 0600. No secret is left in `lull.db`. |

macOS isn't built, so nothing is claimed for it. The Keychain store would plug into the same seam.

### 10. How it lands

Behind the switch `secrets` (ADR-0096), except where a piece only makes today's files safer. That kind of piece lands as a fix:

1. **Fixes, unswitched:**
   - slskd's folder and `slskd.yml` created 0700 and 0600;
   - the data folder lull creates made 0700 where lull creates it, and `lull.db` and the backups 0600;
   - Android's `settings.json` excluded from backups;
   - `lull config set` taking secrets by prompt or stdin (delivery's area: agreed with them first).
2. The `lull_core::secrets` seam, the file vault and `secrets_kept`. Nothing uses them while the switch is off.
3. `core_config` secrets read and written through the seam, and the move.
4. The slskd login through the environment.
5. The Last.fm session key and the Subsonic passwords, each in its owner's crate (additive).
6. The keyring vault in the app (Windows, Linux) and the app's device token, then Android's.
7. The "where it's kept" line in Settings.
8. The canary test and the two Linux CI jobs. They land with pieces 2 and 6, not last.

Finishing the work removes the switch and the plain-row read path, except the move itself. The move stays for upgrades from older versions.

## Alternatives

- **Encrypting the secrets, or the whole database, with a key kept beside them:** the key sits next to the lock. That protects nothing beyond file modes, with more code to get wrong.
- **SQLCipher with its key in the keyring:** it would protect history too. But it is a heavy C dependency, it is all or nothing, and the server has no keyring to hold the key.
- **`systemd-creds` / `LoadCredentialEncrypted` for the package:** encrypting needs root, but the web page changes secrets at run time as `lull`. It could later serve credentials the admin sets (`$CREDENTIALS_DIRECTORY`), beside the environment.
- **Tauri's Stronghold plugin:** its vault needs a password from somewhere, which is the same problem again.
- **Falling back to the file whenever the keyring fails:** it silently lowers protection, and a locked keyring can't be read to move from anyway.
- **Keeping `slskd.yml` and only making it 0600:** it works, but it keeps a second copy of the password on disk.
- **`oo7`, GNOME's Secret Service client:** a good Linux-only choice, and it reaches the Flatpak portal. lull isn't a Flatpak, and `keyring-core` gives one API across Windows, Linux and Android.
- **The all-in-one `keyring` crate:** it brings every platform's store and a feature matrix. Using `keyring-core` with one store per target keeps each build to its own platform.

## Consequences

- The Linux package and Docker get file modes, not encryption. Settings says so in those words.
- A listener whose keyring stays locked has a Soulseek or Jellyfin that waits for them, with a message, instead of one that silently stops working.
- Going back to an older version means typing the passwords again. Nothing breaks.
- Support requests can include `lull.db`, the backups, `settings.json` and `slskd.yml` without leaking a login. Only the file vault, where it is used, stays private.
