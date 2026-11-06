#!/bin/sh
# After the .deb or .rpm is installed or upgraded: the service's own user, then the service, running.
set -e
if ! getent passwd lull >/dev/null; then
  useradd --system --user-group --home-dir /var/lib/lull --no-create-home --shell /usr/sbin/nologin \
    --comment "lull, the music hub" lull
fi
if [ -d /run/systemd/system ]; then
  systemctl daemon-reload
  systemctl enable lull.service >/dev/null 2>&1
  systemctl restart lull.service
  echo "lull runs. Set it up with: sudo lull setup"
else
  echo "lull is installed, but this system has no systemd: start it with /usr/lib/lull-server/lull serve (ADR-0090)."
fi
