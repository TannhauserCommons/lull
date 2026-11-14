#!/bin/sh
# ADR-0091 on Linux with a Secret Service: the keyring tests against GNOME Keyring in its own D-Bus session, unlocked,
# then locked. Linux only (Debian packages).
set -eu
cd "$(dirname "$0")/../.."
. scripts/ci/lib.sh

apt_install $LINUX_DEPS gnome-keyring dbus-x11
cd app
cargo test --locked --no-run
# The daemon reads its password from stdin, makes the login collection and unlocks it. Locking it afterwards needs no
# prompt; reading it then asks for one, which nobody can answer here.
dbus-run-session -- sh -euc '
  printf lull-ci | gnome-keyring-daemon --components=secrets --daemonize --unlock >/dev/null
  LULL_TEST_KEYRING=present cargo test --locked keyring
  dbus-send --session --print-reply --dest=org.freedesktop.secrets /org/freedesktop/secrets \
    org.freedesktop.Secret.Service.Lock array:objpath:/org/freedesktop/secrets/collection/login >/dev/null
  LULL_TEST_KEYRING=locked timeout 120 cargo test --locked keyring
'
