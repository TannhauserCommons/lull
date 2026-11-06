#!/bin/sh
# Before the .deb or .rpm is removed (not upgraded): the service stops and won't start at boot. The data in
# /var/lib/lull and the music stay.
set -e
remove=false
case "$1" in remove | purge | 0) remove=true ;; esac
if $remove && [ -d /run/systemd/system ]; then
  systemctl disable --now lull.service >/dev/null 2>&1 || true
fi
